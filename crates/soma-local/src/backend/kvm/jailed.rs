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
mod checkpoint;
mod command;
mod control;
mod file;
mod launch;
mod outcome;
mod pause;
mod pty;
mod restore_inputs;
mod workspace;

use std::{
    fs::File,
    time::{Duration, Instant},
};

use soma::{BackendFailureKind, CleanupMethod, InstanceId};
use soma_guest::LaunchNetwork;
use soma_jail::{JailHandle, ProbeReport};
use soma_vmm::control::{Outcome, Request};
use soma_vmm::sandbox::{ActivationChannel, PendingActivation};
use soma_vmm::{
    ContractVersion, DeclaredDevices, DiskBytes, Generation, GenerationId, HotplugBytes,
    InstanceId as VmmInstanceId, Launch, MachineSpec, MemoryBytes, OperationId, Stop, VcpuCount,
};

pub(super) use anchors::Anchors;

use super::identity::{fresh16, instance_bytes};
use super::network::Egress;
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
    pub(super) instance: &'a InstanceId,
    pub(super) operation: &'a soma::OperationId,
    pub(super) instance_bytes: [u8; 16],
    pub(super) generation_bytes: [u8; 32],
    pub(super) memory_mib: u64,
    pub(super) disk_mib: u64,
    pub(super) checkpoint: Option<(&'a InstanceId, &'a soma::OperationId)>,
    pub(super) volume: Option<&'a soma::VolumeAttachment>,
}

/// The network identity bound by the supervisor to an already claimed TAP.
pub(super) struct NetworkLaunch<'a> {
    pub(super) launch: LaunchNetwork,
    pub(super) pending: PendingActivation,
    pub(super) operation: [u8; 16],
    pub(super) egress: &'a mut Egress,
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
        network: Option<(File, NetworkLaunch<'_>)>,
    ) -> Result<Self, BackendFailureKind> {
        let devices = launching.prepared.manifest.device_set();
        if network.is_some() && !devices.net() {
            return Err(BackendFailureKind::WorkloadRejected);
        }
        let (tap, activation) = network.map_or((None, None), |(tap, activation)| {
            (Some(tap), Some(activation))
        });
        let checkpoint = launching
            .checkpoint
            .map(|(source, checkpoint)| {
                super::checkpoint::restore_artifacts(source, &launching.prepared.id, checkpoint)
                    .map_err(|error| match error {
                        crate::CheckpointFailure::NotFound => BackendFailureKind::WorkloadRejected,
                        crate::CheckpointFailure::Unsupported => BackendFailureKind::Unsupported,
                        _ => BackendFailureKind::Unavailable,
                    })
            })
            .transpose()?;
        if let Some(artifacts) = checkpoint.as_ref() {
            use soma_kvm::contract::MachineContract;
            use soma_kvm::snapshot::compatibility::Incompatibility;
            use soma_kvm::x86_64::{SnapshotError, check_checkpoint_cpu};

            let manifest = &launching.prepared.manifest;
            let contract = MachineContract::require(manifest.machine_contract.version)
                .map_err(|_| BackendFailureKind::WorkloadRejected)?;
            check_checkpoint_cpu(artifacts.cpu_template, contract, manifest.shape.vcpu_count)
                .map_err(|error| {
                    anchors.record(
                        launching.instance.as_str(),
                        &format!("checkpoint CPU preflight refused: {error}"),
                    );
                    match error {
                        SnapshotError::Incompatible(Incompatibility::CpuTemplate { .. }) => {
                            BackendFailureKind::CheckpointIncompatible
                        }
                        _ => BackendFailureKind::Unavailable,
                    }
                })?;
        }
        let mut recovery_volume = checkpoint
            .as_ref()
            .and_then(|artifacts| artifacts.volume.as_ref())
            .map(|receipt| super::jailed_volume::recover(launching, receipt))
            .transpose()?;
        let (volume, volume_bytes) = super::jailed_volume::open(
            launching,
            recovery_volume.as_ref().map(|volume| &volume.attachment),
        )?;
        let spec = build::spec(launching, volume_bytes, tap.is_some())?;
        let opened = build::open(anchors, launching, checkpoint, volume, tap)?;
        let handle =
            soma_jail::launch(&spec, &anchors.host, opened.resources).map_err(|error| {
                anchors.record(
                    launching.instance.as_str(),
                    &format!("jail launch refused: {error}"),
                );
                BackendFailureKind::IsolationFailure
            })?;
        let mut jailed = Self {
            handle: Some(handle),
            control: control::Control::adopt(opened.control),
            instance: VmmInstanceId::new(launching.instance_bytes)
                .map_err(|_| BackendFailureKind::WorkloadRejected)?,
            attestation: outcome::UNATTESTED,
            poisoned: false,
        };
        jailed.attestation = jailed.admit(anchors, launching.instance)?;
        let mut declared = DeclaredDevices::new(devices.overlay(), devices.net());
        if let Some(capacity) = volume_bytes {
            declared = declared.with_volume_capacity(capacity);
        }
        let activation = match (opened.activation, activation) {
            (None, None) => None,
            (Some(channel), Some(network)) => Some((ActivationChannel::new(channel), network)),
            _ => return Err(BackendFailureKind::IsolationFailure),
        };
        jailed.perform_launch(anchors, launching, declared, opened.recovery, activation)?;
        if let Some(workspace) = &mut recovery_volume {
            workspace.publish()?;
        }
        Ok(jailed)
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

    /// A handle over a control socket whose far end a test plays, with no jail behind it.
    #[cfg(test)]
    pub(super) fn for_test(socket: soma_jail::ControlSocket) -> Self {
        Self {
            handle: None,
            control: control::Control::adopt(socket),
            instance: VmmInstanceId::new([2; 16]).expect("instance"),
            attestation: ProbeReport {
                pid: 1,
                uid: 0,
                euid: 0,
                gid: 0,
                egid: 0,
                table_sealed: true,
                first_bad_slot: None,
                root: soma_jail::RootView {
                    entries: 0,
                    writable: false,
                    proc_visible: false,
                    sys_visible: false,
                },
            },
            poisoned: false,
        }
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

#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
mod streaming_tests;
