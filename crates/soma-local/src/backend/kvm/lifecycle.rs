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
    BackendFailure, BackendFailureKind, CommandStatus, EffectiveNetwork, InstanceId, MachineShape,
    OperationId,
};
use soma_vmm::sandbox::Network;

use super::{
    KvmBackend, claim,
    evidence::effective_network,
    fit,
    held::Held,
    host::Launched,
    identity::LaunchIdentity,
    network::Egress,
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
    /// The vCPUs the machine booted with, which is the most a live resize can give it.
    pub(super) vcpus: u16,
    /// The vCPUs the machine is limited to now.
    pub(super) cpu_limit: u16,
    /// The memory the machine booted with, in MiB.
    pub(super) memory_mib: u64,
}

impl KvmBackend {
    /// Builds the one machine this process will hold, and reports what that established.
    pub(super) fn launch_resident(
        &mut self,
        operation: &OperationId,
        instance: &InstanceId,
        prepared: &PreparedGeneration,
        shape: &MachineShape,
        volume: Option<soma::VolumeAttachment>,
    ) -> Result<Launched, BackendFailure> {
        // This Backend owns at most one sandbox. Assigning over a live one would drop its
        // Session, and dropping a Session shuts the guest down, so launching B would silently
        // destroy A without any Stop or Cleanup naming A.
        if self.live.is_some() {
            return Err(self.fail(operation, BackendFailureKind::ResourceConflict));
        }
        // A request the machine cannot give is refused rather than served by a machine that is
        // not the shape asked for; the bounds come from the Generation it is built with.
        let booted_vcpus = prepared.manifest.shape.vcpu_count;
        let boot_mib = prepared.manifest.shape.memory_bytes / (1024 * 1024);
        if !self.admits(prepared, shape) {
            return Err(self.fail(operation, BackendFailureKind::WorkloadRejected));
        }
        let Some(storage_mib) = prepared
            .manifest
            .template
            .writable_storage_bytes
            .checked_div(1024 * 1024)
            .filter(|_| {
                prepared
                    .manifest
                    .template
                    .writable_storage_bytes
                    .is_multiple_of(1024 * 1024)
            })
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
        let has_device = prepared.manifest.device_set().net();
        let mut egress = Egress::claim(self.broker.as_deref(), policy, identity, has_device)
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
        // and the next one for the same Generation finds a machine already restored. An empty
        // pool is not a failure and is never reported as a prepared launch. A machine prepared
        // inside this process is the unjailed thing the split removes, so a host that jails its
        // machines never claims one.
        let claimed = if self.jail.is_some() {
            None
        } else {
            let key = claim::recipe_for(prepared, boot_mib).map(|recipe| recipe.key().clone());
            let primed_matches = self
                .primed
                .as_ref()
                .is_some_and(|(prepared_key, _, _)| Some(prepared_key) == key.as_ref());
            if primed_matches {
                self.primed
                    .take()
                    .map(|(_, session, overlay)| claim::ClaimedMachine::Primed { session, overlay })
            } else {
                claim::prepare_and_claim(&self.machines, prepared, boot_mib)
            }
        };
        // No secret reaches this Backend yet: the portable request carries a Template's secret
        // references, not values, and the host side that resolves one does not exist. The
        // placement is wired, so a launch given a secret it cannot place fails here.
        let launching = Launching {
            instance,
            prepared,
            identity,
            memory_mib: boot_mib,
            storage_mib: shape.storage_mib(),
            network,
            secrets: Vec::new(),
            // The volume the host resolved, when the Generation declares one; a machine given
            // none refuses to start rather than serve an empty disk.
            volume,
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
            vcpus: booted_vcpus,
            cpu_limit: booted_vcpus,
            memory_mib: boot_mib,
        });
        // The answer states the machine as it is, so a Launch that cannot give the limits it
        // was asked for ends here instead of reporting a shape the machine does not have.
        if let Err(kind) = self.fit_to_request(instance, shape) {
            let _ignored = self.cleanup_resident(instance, Force::Immediately);
            return Err(self.fail(operation, kind));
        }
        Ok(Launched {
            preparation,
            memory_mib: shape.memory_mib(),
            storage_mib,
            vcpus: shape.vcpu_count(),
            network: observed,
            at_ns: launched,
            live_cgroup: None,
        })
    }
}

#[path = "lifecycle/lookup.rs"]
pub(super) mod lookup;
mod resident;

impl KvmBackend {
    /// Whether this Backend can give a request the vCPUs and memory it names.
    ///
    /// A request may be smaller than the machine the Generation booted: the vCPUs it did not
    /// ask for are withheld by a CPU quota applied before the launch answers. It can never be
    /// larger, because the guest has only the processors it booted with. Memory is restored at
    /// the Generation's own boot size and grown to the request, which needs the memory device
    /// and a machine that is not jailed.
    fn admits(&self, prepared: &PreparedGeneration, shape: &MachineShape) -> bool {
        fit::request_fits(prepared, shape)
            && (self.jail.is_none() || shape.memory_mib() == fit::boot_mib_of(prepared))
    }
}
