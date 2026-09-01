//! Launch, execute, inspect, and release for one KVM sandbox, in the process that holds it.
//!
//! Everything here is resident work: it touches the machine, its memory mapping, its overlay
//! head, its network lease, and its authenticated session, all of which belong to one process.
//! The Backend holds at most one live sandbox at a time, keyed by the Instance that launched it,
//! because one process drives one sandbox from launch to release. A request naming a different
//! Instance is refused rather than served by the wrong machine.
//!
//! Which process performs this work is decided one level up, in `dispatch`. A `soma run` does it
//! in the command's own process; a `soma machine launch` starts a host that does it and stays.
//! Neither changes what happens here, so the per-Instance identity, the sterile assignment, and
//! the Noise session are established exactly once either way.

use std::time::Duration;

use soma::{
    BackendFailure, BackendFailureKind, CleanupDisposition, CleanupEvidence, CommandStatus,
    EffectiveNetwork, InstanceId, MachineShape, MachineState, OperationId,
};
use soma_guest::GuestCommand;
use soma_vmm::sandbox::Network;

use super::{
    KvmBackend, claim,
    evidence::{CONTRACT_VCPUS, command_status, effective_network},
    held::Held,
    host::Launched,
    identity::LaunchIdentity,
    network::{Egress, Released},
    prepared::PreparedGeneration,
    start::{Launching, Started},
};

/// How long one bounded command may take before the session is considered gone.
pub(in crate::backend::kvm) const COMMAND_CEILING: Duration = Duration::from_secs(300);

/// Whether a release may end the machine without asking the guest.
#[derive(Clone, Copy)]
pub(super) enum Force {
    /// End the machine now. This is what a forced destroy is.
    Immediately,
    /// Ask the guest to shut down, and end the machine when it will not.
    OnlyIfTheGuestWillNotLeave,
}

/// One command that ran, as the portable lifecycle reports it.
pub(super) type Ran = (CommandStatus, Vec<u8>, Vec<u8>);

/// The live sandbox this Backend is driving.
pub(super) struct Live {
    pub(super) instance: InstanceId,
    /// Whichever process holds this Instance's machine.
    pub(super) held: Held,
    /// The network this Instance holds, released when its sandbox is.
    pub(super) egress: Egress,
    /// What this Instance was told its network is, reported again by every later observation.
    pub(super) network: EffectiveNetwork,
}

impl KvmBackend {
    /// Builds the one machine this process will hold, and reports what that established.
    pub(super) fn launch_resident(
        &mut self,
        operation: &OperationId,
        instance: &InstanceId,
        prepared: &PreparedGeneration,
        shape: &MachineShape,
    ) -> Result<Launched, BackendFailure> {
        // This Backend owns at most one sandbox. Assigning over a live one would drop its
        // Session, and dropping a Session shuts the guest down, so launching B would silently
        // destroy A without any Stop or Cleanup naming A.
        if self.live.is_some() {
            return Err(self.fail(operation, BackendFailureKind::ResourceConflict));
        }
        // The machine contract fixes one vCPU, so a larger shape is refused rather than
        // silently served by a machine that is not the shape the caller asked for.
        if shape.vcpu_count() != CONTRACT_VCPUS {
            return Err(self.fail(operation, BackendFailureKind::WorkloadRejected));
        }
        let Some(storage_mib) = prepared
            .manifest
            .template
            .writable_storage_bytes
            .checked_div(1024 * 1024)
            .filter(|_| prepared.manifest.template.writable_storage_bytes % (1024 * 1024) == 0)
        else {
            return Err(self.fail(operation, BackendFailureKind::Unavailable));
        };
        if shape.storage_mib() != storage_mib {
            return Err(self.fail(operation, BackendFailureKind::WorkloadRejected));
        }
        let identity =
            LaunchIdentity::derive(instance).map_err(|kind| self.fail(operation, kind))?;
        let policy = shape.capabilities().network_policy();
        // A request that needs egress is served by the broker or refused. It is never served by
        // the placeholder device, which would report a working network that drops every packet.
        let mut egress = Egress::claim(self.broker.as_deref(), policy, identity)
            .map_err(|kind| self.fail(operation, kind))?;
        let network = Network {
            launch: egress
                .launch(identity)
                .map_err(|kind| self.fail(operation, kind))?,
            attachment: egress.attachment(),
            activation: egress.pending_activation(),
        };
        let observed = effective_network(&egress, policy);
        // Preparation is registered before the claim, so this Launch may find nothing prepared
        // and the next one for the same Generation finds a machine that is already restored.
        // An empty pool is not a failure and is never reported as a prepared launch: the Launch
        // restores its own machine on the path below and says so.
        // A machine prepared inside this process is exactly the unjailed thing the split
        // removes, so a host that jails its machines never claims one.
        let claimed = if self.jail.is_some() {
            None
        } else {
            claim::prepare_and_claim(&self.machines, prepared, shape.memory_mib())
        };
        // No secret reaches this Backend yet. The portable Launch request carries a Template's
        // secret references, not their values, and the host side that resolves a reference into
        // a value is the credential mediator of the second delivery mode, which does not exist.
        // The placement itself is wired, so a launch given a secret it cannot place fails here
        // rather than running without it.
        let launching = Launching {
            instance,
            prepared,
            identity,
            memory_mib: shape.memory_mib(),
            storage_mib: shape.storage_mib(),
            network,
            secrets: Vec::new(),
        };
        let Started {
            preparation,
            held,
            launched,
        } = if self.jail.is_some() {
            self.launch_in_a_jail(operation, launching)?
        } else {
            match claimed {
                Some(claimed) => self.assign_claimed(operation, launching, &mut egress, claimed)?,
                None => self.restore_on_demand(operation, launching, &mut egress)?,
            }
        };
        self.live = Some(Live {
            instance: instance.clone(),
            held,
            egress,
            network: observed.clone(),
        });
        Ok(Launched {
            preparation,
            memory_mib: shape.memory_mib(),
            storage_mib,
            network: observed,
            at_ns: launched,
        })
    }

    /// Runs one bounded command over the authenticated session this process holds.
    pub(super) fn execute_resident(
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

    /// Reports the state of the sandbox this process is driving.
    pub(super) fn inspect_resident(
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
    pub(super) fn cleanup_resident(
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
    pub(super) fn release_unowned(
        &mut self,
        instance: &InstanceId,
    ) -> Result<CleanupEvidence, BackendFailureKind> {
        self.ownership.release(instance)?;
        Ok(lookup::not_owned_evidence())
    }
}

#[path = "lifecycle/lookup.rs"]
pub(super) mod lookup;
