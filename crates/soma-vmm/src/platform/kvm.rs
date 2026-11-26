//! The KVM platform: one restored machine, held by a jailed worker.
//!
//! This is the provider behind the lifecycle contract. It owns nothing the jail forbids: every
//! resource arrives as a sealed descriptor, the machine is restored from open handles, and the
//! only conversation it has is the one its supervisor drives over the control socket.
//!
//! The two halves of a Launch map onto the two halves of a restore that already exist. A
//! sterile machine is restored and parked first, which is the work that does not depend on
//! which Instance it will serve; the Instance's own authority, its private head, its context
//! identifier and its launch material are transferred second. So a failure in either half is
//! reported as the half it happened in rather than as one indivisible launch.

mod command;
mod faults;
use faults::left_on_its_own;
mod generation;
mod network;
use command::guest_command;
mod recovery;
mod resources;
use network::network_for_cid;

use std::fs::File;
use std::time::Duration;

use soma_guest::GuestCommand;
use soma_jail::DescriptorManifest;
use soma_kvm::memory_layout::{GuestLayout, HotplugRegion};
use soma_kvm::x86_64::{Hypervisor, NetworkAttachment, SnapshotObjects};
use soma_kvm::{DeviceSet, MachineContract};

use crate::sandbox::{
    ActivationChannel, Assignment, AssignmentVolume, Network, Session, SessionError, SterileSpec,
    guest_cid_for, link_down_network,
};
use crate::{Execute, Launch, Recovery, Stop};

use super::{
    Platform, PlatformExecution, PlatformFailure, PlatformStop, ReadinessFailure,
    ReadyAuthenticatedGuest, RestoreFailure, RestoreProgress, RestoreStep, RestoredMachine,
};

pub(crate) use resources::MachineResources;

/// How much longer than its own deadline one command may take before the session is gone.
///
/// The guest enforces the command's timeout itself and answers with `TimedOut`, so this bounds
/// a session that has stopped answering at all rather than the command.
const COMMAND_SLACK: Duration = Duration::from_secs(30);

/// One machine, from the descriptors it was given to the evidence it leaves.
pub(crate) struct KvmPlatform {
    /// Taken by the restore; `None` afterwards, so a second Launch cannot rebuild the machine.
    resources: Option<MachineResources>,
    /// This Instance's private head, held between the restore and the assignment that installs
    /// it. A sterile machine must not be holding one Instance's writable storage.
    overlay: Option<File>,
    volume: Option<File>,
    network: Option<(File, ActivationChannel)>,
    recovery: Option<recovery::Selection>,
    recovered: bool,
    checkpoint_geometry: Option<([u8; 32], MachineContract, bool, bool)>,
    /// The parked or serving sandbox.
    session: Option<Session>,
}

impl KvmPlatform {
    /// Adopts the sealed descriptor table, or `None` when it does not name a whole machine.
    pub(crate) fn adopt(manifest: &DescriptorManifest) -> Option<Self> {
        Some(Self {
            resources: Some(MachineResources::adopt(manifest)?),
            overlay: None,
            volume: None,
            network: None,
            session: None,
            checkpoint_geometry: None,
            recovery: None,
            recovered: false,
        })
    }
}

impl Platform for KvmPlatform {
    fn configure_recovery(
        &mut self,
        digest: soma_kvm::snapshot::Digest,
        saved_overlay: Option<File>,
        fork_from: Option<crate::InstanceId>,
        memory: soma_kvm::snapshot::RestoreAttestations,
    ) -> Result<(), PlatformFailure> {
        if digest.is_zero() || self.resources.is_none() || self.recovery.is_some() {
            return Err(PlatformFailure::new(Recovery::DoNotRetry));
        }
        self.recovery = Some(recovery::Selection {
            digest,
            saved_overlay,
            fork_from,
            memory,
        });
        Ok(())
    }

    fn verify_and_restore(&mut self, launch: &Launch) -> Result<RestoredMachine, RestoreFailure> {
        self.restore_generation(launch)
    }

    fn authenticate_repair_and_ready(
        &mut self,
        launch: &Launch,
        _restored: RestoredMachine,
    ) -> Result<ReadyAuthenticatedGuest, ReadinessFailure> {
        if self.recovered && self.session.is_some() {
            return Ok(ReadyAuthenticatedGuest::authenticated());
        }
        let overlay = self.overlay.take();
        let Some(session) = self.session.as_mut() else {
            return Err(faults::readiness(SessionError::Create));
        };
        let instance = *launch.instance_id().as_bytes();
        let guest_cid = guest_cid_for(instance);
        let (network, activation) =
            network_for_cid(self.network.take(), guest_cid).map_err(faults::readiness)?;
        let assignment = Assignment {
            overlay,
            volume: self.volume.take().map(AssignmentVolume::Descriptor),
            generation: *launch.generation().id().as_bytes(),
            instance,
            operation: *launch.operation_id().as_bytes(),
            guest_cid,
            network,
            secrets: Vec::new(),
        };
        match session.assign(assignment, &mut |receipt| {
            activation
                .as_ref()
                .ok_or(SessionError::Network)?
                .ask(receipt)
        }) {
            Ok(()) => Ok(ReadyAuthenticatedGuest::authenticated()),
            Err(error) => Err(faults::readiness(error)),
        }
    }

    fn execute(
        &mut self,
        execute: &Execute,
        guest: &mut ReadyAuthenticatedGuest,
    ) -> Result<PlatformExecution, PlatformFailure> {
        self.execute_streaming(execute, guest, &mut |_, _| {})
    }

    fn execute_streaming(
        &mut self,
        execute: &Execute,
        _guest: &mut ReadyAuthenticatedGuest,
        sink: &mut dyn FnMut(crate::control::OutputStream, &[u8]),
    ) -> Result<PlatformExecution, PlatformFailure> {
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| PlatformFailure::new(Recovery::RepairHost))?;
        let limits = execute.limits();
        let command = guest_command(execute)?;
        let deadline = Duration::from_millis(u64::from(limits.timeout().get())) + COMMAND_SLACK;
        let completed = session
            .execute_streaming(command, deadline, |source, bytes| {
                sink(
                    match source {
                        soma_guest::OutputSource::Stdout => crate::control::OutputStream::Stdout,
                        soma_guest::OutputSource::Stderr => crate::control::OutputStream::Stderr,
                    },
                    bytes,
                );
            })
            .map_err(|error| PlatformFailure::new(faults::execute_recovery(error)))?;
        let status = faults::exit_status(completed.status)
            .ok_or_else(|| PlatformFailure::new(Recovery::ReplaceMachine))?;
        Ok(PlatformExecution::completed(
            status,
            completed.stdout,
            completed.stderr,
        ))
    }

    fn file(
        &mut self,
        operation: soma::FileOperation,
    ) -> Result<soma::FileAnswer, PlatformFailure> {
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| PlatformFailure::new(Recovery::RepairHost))?;
        session
            .file(operation)
            .map_err(|error| PlatformFailure::new(faults::execute_recovery(error)))
    }

    fn pty(&mut self, operation: &soma::PtyOperation) -> Result<soma::PtyAnswer, PlatformFailure> {
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| PlatformFailure::new(Recovery::RepairHost))?;
        session
            .pty(operation.clone())
            .map_err(|error| PlatformFailure::new(faults::execute_recovery(error)))
    }

    fn checkpoint(
        &mut self,
        job: crate::sandbox::CheckpointJob,
        deadline: Duration,
    ) -> Result<soma_kvm::snapshot::Digest, crate::sandbox::CheckpointJobError> {
        if self.checkpoint_geometry
            != Some((
                job.source.candidate_id,
                job.source.contract,
                job.source.has_overlay,
                job.source.has_volume,
            ))
        {
            return Err(crate::sandbox::CheckpointJobError::CaptureRefused);
        }
        self.session
            .as_mut()
            .ok_or(crate::sandbox::CheckpointJobError::SourceUnconfirmed)?
            .checkpoint(job, deadline)
            .map(|manifest| soma_kvm::snapshot::Digest::of(&manifest.encode()))
    }

    fn change_workspace(
        &mut self,
        change: crate::sandbox::WorkspaceChange,
    ) -> Result<soma_guest::WorkspaceOutcome, PlatformFailure> {
        self.session
            .as_mut()
            .ok_or_else(|| PlatformFailure::new(Recovery::RepairHost))?
            .change_workspace(change)
            .map_err(|error| PlatformFailure::new(faults::execute_recovery(error)))
    }

    fn memory(&mut self, requested: Option<u64>) -> faults::MemoryAnswer {
        faults::memory(self.session.as_mut(), requested)
    }

    fn pause(&mut self) -> Result<(), PlatformFailure> {
        self.session
            .as_mut()
            .ok_or_else(|| PlatformFailure::new(Recovery::RepairHost))?
            .pause()
            .map_err(|error| PlatformFailure::new(faults::execute_recovery(error)))
    }

    fn resume(&mut self) -> Result<(), PlatformFailure> {
        self.session
            .as_mut()
            .ok_or_else(|| PlatformFailure::new(Recovery::RepairHost))?
            .resume()
            .map_err(|error| PlatformFailure::new(faults::execute_recovery(error)))
    }

    fn stop(
        &mut self,
        _stop: &Stop,
        _guest: Option<&mut ReadyAuthenticatedGuest>,
    ) -> Result<PlatformStop, PlatformFailure> {
        self.overlay = None;
        let Some(session) = self.session.take() else {
            // No machine was ever built, so nothing is left to release.
            return Ok(PlatformStop::released(false, false));
        };
        if !session.is_usable() {
            // The session already ended without a certain answer, and dropping it releases the
            // machine, so the guest was never asked and the machine is ended for it.
            drop(session);
            return Ok(PlatformStop::released(false, true));
        }
        match session.shutdown() {
            Ok(evidence) => {
                let acknowledged = left_on_its_own(&evidence.exit);
                Ok(PlatformStop::released(acknowledged, !acknowledged))
            }
            // The shutdown exchange did not complete, and the sandbox thread finished the
            // machine on its way out, so the machine is gone and the receipt says the stop was
            // forced rather than claiming the guest agreed to it.
            Err(_) => Ok(PlatformStop::released(false, true)),
        }
    }

    fn rollback(&mut self, _launch: &Launch) -> Result<(), PlatformFailure> {
        // Dropping the session ends the sandbox thread, which finishes the machine before it
        // returns, so the VM, its mapping, and every descriptor it owns are released here.
        drop(self.session.take());
        drop(self.resources.take());
        drop(self.overlay.take());
        Ok(())
    }
}
