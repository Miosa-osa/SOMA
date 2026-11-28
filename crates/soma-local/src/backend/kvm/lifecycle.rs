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
    pub(super) workspace_offline: bool,
    pub(super) paused: bool,
    pub(super) generation: soma::GenerationId,
    pub(super) checkpoint_source: soma_vmm::sandbox::CheckpointSource,
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
    /// Whether this machine holds a bundle whose egress may be opened and closed.
    pub(super) windowed: bool,
}

/// What a launch carries besides the Generation and the shape.
///
/// Gathered so that adding one more fact does not add one more argument to every launch.
pub(in crate::backend::kvm) struct LaunchExtras<'a> {
    /// The durable volume the host resolved, when the Generation declares one.
    pub(in crate::backend::kvm) volume: Option<soma::VolumeAttachment>,
    /// Whether the machine must hold a network whose egress can be opened after launch.
    pub(in crate::backend::kvm) windowed: bool,
    pub(in crate::backend::kvm) checkpoint: Option<(&'a InstanceId, &'a OperationId)>,
    /// The decrypted Suspension this launch resumes instead of the Generation's snapshot.
    pub(in crate::backend::kvm) resume: Option<std::path::PathBuf>,
}

impl KvmBackend {
    /// Starts an operation's clock, if nothing has started it.
    ///
    /// A machine host calls this the moment it receives a launch, before it does any of the work
    /// that launch asks for. Without it the clock starts at the first milestone that reads it,
    /// which is the launch stamp itself: the machine would appear to exist the instant it was
    /// asked to start, and everything spent building it would be charged to the readiness that
    /// follows. See [`crate::backend::clock::OperationClocks::start`].
    pub(in crate::backend::kvm) fn begin_operation(&mut self, operation: &OperationId) {
        let _started = self.clocks.start(operation);
    }

    /// Builds the one machine this process will hold, and reports what that established.
    #[allow(clippy::too_many_lines)]
    pub(super) fn launch_resident(
        &mut self,
        operation: &OperationId,
        instance: &InstanceId,
        prepared: &PreparedGeneration,
        shape: &MachineShape,
        extras: LaunchExtras<'_>,
    ) -> Result<Launched, BackendFailure> {
        let (volume, mut scratch_guard) =
            super::scratch_volume::resolve(&extras, instance, prepared)
                .map_err(|kind| self.fail(operation, kind))?;
        let LaunchExtras {
            windowed,
            checkpoint,
            resume,
            ..
        } = extras;
        // A jailed worker is given descriptors by a protocol with no Suspension: no resume there.
        if resume.is_some() && (self.jail.is_some() || checkpoint.is_some()) {
            return Err(self.fail(operation, BackendFailureKind::Unsupported));
        }
        if checkpoint.is_some_and(|(source, _)| {
            self.jail.is_none()
                || volume.is_some()
                || windowed
                || (source != instance
                    && *shape.capabilities().network_policy() != soma::NetworkPolicy::isolated())
        }) {
            return Err(self.fail(operation, BackendFailureKind::Unsupported));
        }
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
        if let Some(reason) = self.misfit(prepared, shape) {
            return Err(self.fail(operation, fit::refuse_shape(instance, &reason)));
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
        let identity =
            LaunchIdentity::derive(instance).map_err(|kind| self.fail(operation, kind))?;
        let checkpoint_source = soma_vmm::sandbox::CheckpointSource {
            instance_id: identity.instance,
            candidate_id: super::identity::generation_bytes(&prepared.id)
                .map_err(|kind| self.fail(operation, kind))?,
            root_digest: soma_kvm::snapshot::Digest::from_bytes(
                *prepared.manifest.root.descriptor.digest.as_bytes(),
            ),
            contract: soma_kvm::MachineContract::require(
                prepared.manifest.machine_contract.version,
            )
            .map_err(|_| self.fail(operation, BackendFailureKind::Unsupported))?,
            has_overlay: prepared.manifest.device_set().overlay(),
            has_volume: volume.is_some()
                || (prepared.manifest.persistence.enabled && checkpoint.is_some()),
        };
        let policy = shape.capabilities().network_policy();
        // A request that needs egress is served by the broker or refused. It is never served by
        // the placeholder device, which would report a working network that drops every packet.
        let has_device = prepared.manifest.device_set().net();
        let mut egress = Egress::claim(
            self.broker.as_deref(),
            policy,
            identity,
            (has_device, windowed),
        )
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
        // A resume restores its own machine, so it never takes one prepared for the Generation.
        let claimed = if self.jail.is_some() || resume.is_some() || checkpoint.is_some() {
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
            // The volume the host resolved, or the scratch disk (`scratch_volume`).
            volume,
            checkpoint,
            resume,
        };
        let Started {
            preparation,
            held,
            launched,
            head_reads,
        } = if self.jail.is_some() {
            self.launch_in_a_jail(operation, launching, &mut egress)?
        } else {
            match claimed {
                Some(claimed) => self.assign_claimed(operation, launching, &mut egress, claimed)?,
                None => self.restore_on_demand(operation, launching, &mut egress)?,
            }
        };
        self.live = Some(Live {
            instance: instance.clone(),
            held,
            workspace_offline: false,
            paused: false,
            generation: prepared.id.clone(),
            checkpoint_source,
            egress,
            network: observed.clone(),
            vcpus: booted_vcpus,
            cpu_limit: booted_vcpus,
            memory_mib: boot_mib,
            windowed,
        });
        scratch_guard.disarm();
        // The answer states the machine as it is, so a Launch that cannot give the limits it
        // was asked for ends here instead of reporting a shape the machine does not have.
        if let Err(kind) = self.fit_to_request(instance, shape) {
            let _ignored = self.cleanup_resident(instance, Force::Immediately);
            return Err(self.fail(operation, kind));
        }
        Ok(launch_answer(
            preparation,
            shape,
            storage_mib,
            head_reads,
            observed,
            launched,
        ))
    }
}

/// The answer a caller reads, once a machine exists and its shape is confirmed.
///
/// Built apart from the launch because the launch is already long, and this is the one part of it
/// that is a report rather than a step.
fn launch_answer(
    preparation: soma::PreparationClass,
    shape: &MachineShape,
    storage_mib: u64,
    head_reads: Vec<soma_kvm::head_reads::HeadSpan>,
    observed: EffectiveNetwork,
    at_ns: u64,
) -> Launched {
    Launched {
        preparation,
        memory_mib: shape.memory_mib(),
        storage_mib,
        head_reads: head_reads.into_iter().map(Into::into).collect(),
        vcpus: shape.vcpu_count(),
        network: observed,
        at_ns,
        live_cgroup: None,
    }
}

#[path = "lifecycle/lookup.rs"]
pub(super) mod lookup;
mod resident;
mod workspace;
