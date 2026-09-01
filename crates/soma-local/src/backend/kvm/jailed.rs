//! One machine held by a jailed worker, addressed by the broker that built it.
//!
//! This is the other half of the split. The broker keeps the socket, the Generation store, the
//! head directory, and the Instance registry; the machine keeps nothing but the descriptors it
//! was handed, inside its own user, mount, PID, network, IPC, UTS, and cgroup namespaces, under
//! a seccomp filter that kills every syscall a server needs.
//!
//! The handle below is what the resident lifecycle holds in place of an in-process session, and
//! it answers the same three questions: run this command, is it still usable, and release it.

mod anchors;
mod build;
mod command;
mod control;
mod outcome;
mod pty;

use std::time::{Duration, Instant};

use soma::{BackendFailureKind, CleanupMethod, InstanceId};
use soma_jail::{JailHandle, ProbeReport};
use soma_vmm::control::Request;
use soma_vmm::{
    DeclaredDevices, DiskBytes, Generation, GenerationId, InstanceId as VmmInstanceId, Launch,
    MachineSpec, MemoryBytes, OperationId, Stop, VcpuCount,
};

pub(super) use anchors::Anchors;

use super::identity::fresh16;
use super::prepared::PreparedGeneration;

/// How long the worker has to attest its own containment before it is considered lost.
pub(super) const ATTESTATION_CEILING: Duration = Duration::from_secs(30);
/// How long a launch has to reach an authenticated Ready inside the jail.
const LAUNCH_CEILING: Duration = Duration::from_secs(180);
/// How long a release has to finish and prove it.
const STOP_CEILING: Duration = Duration::from_secs(90);
/// How long the reconciliation of a jail may take before it is reported as incomplete.
const RECONCILE_CEILING: Duration = Duration::from_secs(10);
/// Bytes in one mebibyte.
const MIB: u64 = 1024 * 1024;

/// One machine, held inside a jail this broker built.
pub(super) struct Jailed {
    handle: Option<JailHandle>,
    pub(super) control: control::Control,
    pub(super) instance: VmmInstanceId,
    /// The attestation the worker sent before it served anything, kept so that a later
    /// question about this machine's containment is answered by what it proved rather than by
    /// what its broker assumed.
    attestation: ProbeReport,
    /// Set once an exchange ended without a certain answer.
    pub(super) poisoned: bool,
}

/// Everything one jailed launch is built from.
pub(super) struct Launching<'a> {
    pub(super) prepared: &'a PreparedGeneration,
    pub(super) snapshot: &'a std::path::Path,
    pub(super) instance: &'a InstanceId,
    pub(super) instance_bytes: [u8; 16],
    pub(super) generation_bytes: [u8; 32],
    pub(super) memory_mib: u64,
    pub(super) disk_mib: u64,
}

impl Jailed {
    /// Builds the jail, hands it the machine's descriptors, and returns once it is Ready.
    ///
    /// Every failure leaves nothing behind: the jail is reconciled, which kills and reaps the
    /// worker and removes its cgroup leaf and its empty root, and the private head goes with
    /// the worker because nothing on the filesystem ever named it.
    ///
    /// # Errors
    ///
    /// Returns the typed refusal of the launch.
    pub(super) fn launch(
        anchors: &Anchors,
        launching: &Launching<'_>,
    ) -> Result<Self, BackendFailureKind> {
        let devices = launching.prepared.manifest.device_set();
        let spec = build::spec(
            launching.instance,
            launching.memory_mib,
            launching.disk_mib,
            devices.overlay(),
        )?;
        let opened = build::open(
            anchors,
            &launching.prepared.store,
            &launching.prepared.manifest.root.descriptor,
            launching.snapshot,
            launching.instance,
            devices.overlay(),
        )?;
        let handle = soma_jail::launch(&spec, &anchors.host, opened.resources)
            .map_err(|_| BackendFailureKind::IsolationFailure)?;
        let mut jailed = Self {
            handle: Some(handle),
            control: control::Control::adopt(opened.control),
            instance: VmmInstanceId::new(launching.instance_bytes)
                .map_err(|_| BackendFailureKind::WorkloadRejected)?,
            attestation: outcome::UNATTESTED,
            poisoned: false,
        };
        jailed.attestation = jailed.admit(anchors, launching.instance)?;
        jailed.perform_launch(launching, devices.overlay(), devices.net())?;
        Ok(jailed)
    }

    /// Reads and checks the worker's own attestation before anything is asked of it.
    ///
    /// The worker already refuses to serve an attestation that does not describe a jail. This
    /// side checks the same report for the same reason from the other end: a broker that
    /// accepted a worker it could not see the containment of would be trusting the process it
    /// exists to constrain.
    fn admit(
        &mut self,
        anchors: &Anchors,
        instance: &InstanceId,
    ) -> Result<ProbeReport, BackendFailureKind> {
        let text = self.control.receive(ATTESTATION_CEILING)?;
        let pid = self.handle.as_ref().map_or(0, JailHandle::pid);
        anchors.record(instance.as_str(), &format!("jailed pid={pid} {text}"));
        let report =
            ProbeReport::decode(&text).map_err(|_| BackendFailureKind::IsolationFailure)?;
        if outcome::describes_a_jail(&report) {
            Ok(report)
        } else {
            Err(BackendFailureKind::IsolationFailure)
        }
    }

    /// Launches the machine and then narrows the worker's filter to its steady state.
    ///
    /// The order is the whole point of the two phases. Restoring a machine creates event
    /// descriptors, epoll sets, and threads, which the startup filter admits; serving commands
    /// needs none of them, so the filter narrows the moment the machine exists and every
    /// startup-only syscall becomes a kill for the rest of the worker's life.
    fn perform_launch(
        &mut self,
        launching: &Launching<'_>,
        overlay: bool,
        network: bool,
    ) -> Result<(), BackendFailureKind> {
        let request = Request::Launch(Launch::new(
            OperationId::new(fresh16()).map_err(|_| BackendFailureKind::WorkloadRejected)?,
            self.instance,
            Generation::new(
                GenerationId::new(launching.generation_bytes)
                    .map_err(|_| BackendFailureKind::WorkloadRejected)?,
                MachineSpec::new(
                    VcpuCount::new(1).map_err(|_| BackendFailureKind::WorkloadRejected)?,
                    MemoryBytes::new(launching.memory_mib.saturating_mul(MIB))
                        .map_err(|_| BackendFailureKind::WorkloadRejected)?,
                    DiskBytes::new(launching.disk_mib.saturating_mul(MIB).max(MIB))
                        .map_err(|_| BackendFailureKind::WorkloadRejected)?,
                ),
                DeclaredDevices::new(overlay, network),
            ),
        ));
        outcome::ready(&self.control.ask(&request, LAUNCH_CEILING)?)?;
        outcome::sealed(&self.control.ask(&Request::Seal, ATTESTATION_CEILING)?)
    }

    /// Whether this machine may still be addressed.
    pub(super) const fn is_usable(&self) -> bool {
        !self.poisoned
    }

    /// Releases the machine, and proves the jail is gone.
    ///
    /// A graceful stop asks the worker to stop its machine and waits for the receipt. Either
    /// way the jail is reconciled afterwards, so the process, its cgroup leaf, and its empty
    /// root are removed rather than left for something else to find.
    pub(super) fn release(mut self, forced: bool) -> Result<CleanupMethod, BackendFailureKind> {
        let method = if forced || self.poisoned {
            None
        } else {
            self.ask_the_guest_to_leave()
        };
        let disposition = self.reconcile();
        match (method, disposition) {
            (_, false) => Err(BackendFailureKind::CleanupFailure),
            (Some(method), true) => Ok(method),
            (None, true) => Ok(CleanupMethod::Forced),
        }
    }

    fn ask_the_guest_to_leave(&mut self) -> Option<CleanupMethod> {
        let operation = OperationId::new(fresh16()).ok()?;
        let request = Request::Stop(Stop::new(operation, self.instance));
        let reply = self.control.ask(&request, STOP_CEILING).ok()?;
        let acknowledged = outcome::stopped(&reply)?;
        // The worker leaves once its Machine proved cleanup, so nothing is left to shut down.
        let _ignored = self.control.tell(&Request::Shutdown(0));
        Some(if acknowledged {
            CleanupMethod::Graceful
        } else {
            CleanupMethod::GracefulThenForced
        })
    }

    /// Kills, reaps, and removes the jail, and reports whether nothing is left.
    fn reconcile(&mut self) -> bool {
        let Some(handle) = self.handle.take() else {
            return true;
        };
        let (disposition, _evidence) = handle.reconcile(Instant::now() + RECONCILE_CEILING);
        disposition.is_released()
    }

    pub(super) fn poison(&mut self, kind: BackendFailureKind) -> BackendFailureKind {
        self.poisoned = true;
        kind
    }
}

impl Drop for Jailed {
    /// A dropped handle must not leave a jailed machine running.
    fn drop(&mut self) {
        let _ignored = self.reconcile();
    }
}
