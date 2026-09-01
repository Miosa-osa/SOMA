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

mod faults;
mod resources;

use std::fs::File;
use std::time::Duration;

use soma_guest::GuestCommand;
use soma_jail::DescriptorManifest;
use soma_kvm::DeviceSet;
use soma_kvm::x86_64::{GuestExit, Hypervisor, SnapshotObjects};

use crate::sandbox::{
    Assignment, Network, Session, SessionError, SterileSpec, guest_cid_for, link_down_network,
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
    /// The parked or serving sandbox.
    session: Option<Session>,
}

impl KvmPlatform {
    /// Adopts the sealed descriptor table, or `None` when it does not name a whole machine.
    pub(crate) fn adopt(manifest: &DescriptorManifest) -> Option<Self> {
        Some(Self {
            resources: Some(MachineResources::adopt(manifest)?),
            overlay: None,
            session: None,
        })
    }
}

impl Platform for KvmPlatform {
    fn verify_and_restore(&mut self, launch: &Launch) -> Result<RestoredMachine, RestoreFailure> {
        let Some(resources) = self.resources.take() else {
            return Err(RestoreFailure::at_restore(Recovery::DoNotRetry));
        };
        let declared = launch.generation().devices();
        let overlay_capacity_bytes = declared
            .writable_disk()
            .then(|| resources.overlay_capacity_bytes())
            .flatten();
        let MachineResources {
            kvm,
            state,
            memory,
            root,
            overlay,
        } = resources;
        self.overlay = overlay;
        let spec = SterileSpec {
            objects: SnapshotObjects::adopt(state, memory, None),
            hypervisor: Hypervisor::Adopted(kvm),
            root,
            overlay_capacity_bytes,
            memory_bytes: launch.generation().machine().memory().get(),
            devices: DeviceSet::new(declared.writable_disk(), declared.network()),
        };
        match Session::prepare(spec) {
            Ok(session) => {
                self.session = Some(session);
                RestoredMachine::from_observation(RestoreProgress::from_steps([
                    RestoreStep::ArtifactsVerified,
                    RestoreStep::MachineRestored,
                ]))
            }
            // Nothing is re-hashed on this path, so every failure here is in the restore rather
            // than in a verification that never ran.
            Err(error) => Err(RestoreFailure::at_restore(faults::restore_recovery(error))),
        }
    }

    fn authenticate_repair_and_ready(
        &mut self,
        launch: &Launch,
        _restored: RestoredMachine,
    ) -> Result<ReadyAuthenticatedGuest, ReadinessFailure> {
        let overlay = self.overlay.take();
        let Some(session) = self.session.as_mut() else {
            return Err(faults::readiness(SessionError::Create));
        };
        let instance = *launch.instance_id().as_bytes();
        let guest_cid = guest_cid_for(instance);
        let Some(network) = link_down_network(guest_cid) else {
            return Err(faults::readiness(SessionError::Create));
        };
        let assignment = Assignment {
            overlay,
            generation: *launch.generation().id().as_bytes(),
            instance,
            operation: *launch.operation_id().as_bytes(),
            guest_cid,
            network: Network {
                launch: network,
                // A jailed machine is given no frame path and no activation, so its one network
                // device keeps the link-down placeholder it was captured with and carries
                // nothing. An Instance that asked for egress is refused before a jail is built
                // rather than served by a device that drops every packet.
                attachment: None,
                activation: None,
            },
            secrets: Vec::new(),
        };
        match session.assign(assignment, &mut |_receipt| Err(SessionError::Network)) {
            Ok(()) => Ok(ReadyAuthenticatedGuest::authenticated()),
            Err(error) => Err(faults::readiness(error)),
        }
    }

    fn execute(
        &mut self,
        execute: &Execute,
        _guest: &mut ReadyAuthenticatedGuest,
    ) -> Result<PlatformExecution, PlatformFailure> {
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| PlatformFailure::new(Recovery::RepairHost))?;
        let limits = execute.limits();
        let command = GuestCommand::new(
            execute.program().as_bytes().to_vec(),
            execute
                .arguments()
                .iter()
                .map(|argument| argument.as_bytes().to_vec())
                .collect(),
            limits.timeout().get(),
            limits.output().get(),
        )
        .map_err(|_| PlatformFailure::new(Recovery::DoNotRetry))?;
        let deadline = Duration::from_millis(u64::from(limits.timeout().get())) + COMMAND_SLACK;
        let completed = session
            .execute(command, deadline)
            .map_err(|error| PlatformFailure::new(faults::execute_recovery(error)))?;
        let status = faults::exit_status(completed.status)
            .ok_or_else(|| PlatformFailure::new(Recovery::ReplaceMachine))?;
        Ok(PlatformExecution::completed(
            status,
            completed.stdout,
            completed.stderr,
        ))
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

/// Whether the guest ended the machine itself rather than being ended inside it.
fn left_on_its_own<E>(exit: &Result<GuestExit, E>) -> bool {
    matches!(
        exit,
        Ok(GuestExit::Halt | GuestExit::Shutdown | GuestExit::Reset | GuestExit::Sentinel)
    )
}
