//! The resident operations a process performs against the machine it holds.

use soma::{
    BackendFailureKind, CleanupDisposition, CleanupEvidence, EffectiveNetwork, InstanceId,
    MachineState,
};
use soma_guest::GuestCommand;

use crate::backend::kvm::KvmBackend;

use super::super::evidence::command_status;
use super::super::network::Released;
use super::{COMMAND_CEILING, Force, Live, Ran, lookup};

impl KvmBackend {
    /// Runs one bounded command over the authenticated session this process holds.
    pub(in crate::backend::kvm) fn execute_resident(
        &mut self,
        instance: &InstanceId,
        command: GuestCommand,
    ) -> Result<Ran, BackendFailureKind> {
        let Some(live) = self.live_for(instance) else {
            return Err(self.absent_kind(instance));
        };
        let completed = live.held.execute(command, COMMAND_CEILING)?;
        let status = command_status(completed.status).ok_or(BackendFailureKind::GuestFailure)?;
        Ok((status, completed.stdout, completed.stderr))
    }

    /// Runs one bounded command like [`Self::execute_resident`], reporting each output chunk to
    /// `sink` while it runs.
    pub(in crate::backend::kvm) fn execute_resident_streaming(
        &mut self,
        instance: &InstanceId,
        command: GuestCommand,
        sink: soma::OutputSink<'_>,
    ) -> Result<Ran, BackendFailureKind> {
        let Some(live) = self.live_for(instance) else {
            return Err(self.absent_kind(instance));
        };
        let completed = live
            .held
            .execute_streaming(command, COMMAND_CEILING, sink)?;
        let status = command_status(completed.status).ok_or(BackendFailureKind::GuestFailure)?;
        Ok((status, completed.stdout, completed.stderr))
    }

    /// Dials `port` on the guest of the sandbox this process is driving.
    pub(in crate::backend::kvm) fn connect_resident(
        &mut self,
        instance: &InstanceId,
        port: u16,
    ) -> Result<std::os::fd::OwnedFd, crate::TunnelFailure> {
        let Some(live) = self.live_for(instance) else {
            return Err(crate::TunnelFailure::NotFound);
        };
        live.egress.connect_guest(port)
    }

    /// Reports the state of the sandbox this process is driving.
    pub(in crate::backend::kvm) fn inspect_resident(
        &mut self,
        instance: &InstanceId,
    ) -> Result<(MachineState, EffectiveNetwork), BackendFailureKind> {
        // A sandbox this Backend is not driving cannot be observed, and reporting it as stopped
        // would claim knowledge of a machine this process never owned.
        let Some(live) = self.live_for(instance) else {
            return Err(self.absent_kind(instance));
        };
        let network = live.network.clone();
        Ok((MachineState::Ready, network))
    }

    /// Releases everything this process holds for the Instance, the way the caller asked.
    ///
    /// A graceful stop asks the guest to shut down and waits for it. A forced destroy does not
    /// ask: it ends the sandbox thread, which finishes the machine on its way out. The two are
    /// separate paths rather than one path with a flag inside it, because the method a caller is
    /// told about has to be the one that actually happened.
    pub(in crate::backend::kvm) fn cleanup_resident(
        &mut self,
        instance: &InstanceId,
        force: Force,
    ) -> Result<CleanupEvidence, BackendFailureKind> {
        // Cleanup is idempotent, but an Instance this Backend never owned is a different fact
        // from one it owned and released.
        let Some(live) = self.take_live(instance) else {
            return self.release_unowned(instance);
        };
        let Live {
            held, mut egress, ..
        } = live;
        let method = held
            .release(instance, matches!(force, Force::Immediately))
            .ok();
        // The lease is released whether or not the guest shut down cleanly. A machine that is
        // gone has no use for a namespace, a TAP, an address lease, or a port mapping, and
        // leaving them behind is the failure that compounds fastest across many sandboxes.
        let network = match egress.release() {
            Released::Complete => CleanupDisposition::Complete,
            Released::NothingHeld => CleanupDisposition::NotOwned,
            // The broker was asked and could not confirm, so this process must not claim it did.
            Released::Incomplete => CleanupDisposition::Incomplete,
        };
        let Some(method) = method else {
            return Err(BackendFailureKind::CleanupFailure);
        };
        // The machine is gone, so the Host must stop owning the Instance that named it. A
        // record left behind is a leak only reconciliation could find, and an ownership the
        // Host could not prove ended must not be reported as a complete cleanup.
        if !self.ownership.release(instance)? {
            return Err(BackendFailureKind::CleanupFailure);
        }
        // The machine, its memory mapping, the private overlay head, and the Instance authority
        // are all owned by the sandbox thread and released when it ends.
        Ok(CleanupEvidence::new(
            CleanupDisposition::Complete,
            CleanupDisposition::Complete,
            CleanupDisposition::Complete,
            network,
            CleanupDisposition::Complete,
        )
        .with_method(method))
    }

    /// Ends what can be ended for an Instance this process holds no machine for.
    ///
    /// Every disposition stays `NotOwned` because that is what happened: this process held no
    /// machine, no memory, no head, and no guest authority. Only the Host Runtime's ownership of
    /// the identity is ended, which is the one terminal act available here.
    pub(in crate::backend::kvm) fn release_unowned(
        &mut self,
        instance: &InstanceId,
    ) -> Result<CleanupEvidence, BackendFailureKind> {
        self.ownership.release(instance)?;
        Ok(lookup::not_owned_evidence())
    }
}
