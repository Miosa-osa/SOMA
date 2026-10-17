//! How one Launch comes to hold a machine, either way it gets one.
//!
//! A Launch either claims a machine the pool prepared before the request arrived or restores one
//! of its own. Everything after that point is identical: the same registration with the Host
//! Runtime, the same fresh authority, the same mint, broker activation and link gate, and the
//! same withdrawal when the machine never reaches Ready. The arms differ only in where the
//! machine came from, which is exactly what the reported preparation class names.

use soma::{BackendFailure, BackendFailureKind, InstanceId, OperationId, PreparationClass};
use soma_guest::SecretFile;
use soma_vmm::sandbox::{Network, Session, SessionError};

use super::held::Held;
use super::jailed::{Anchors, Jailed, Launching as JailedLaunching};

use super::{
    KvmBackend,
    boot::boot_for,
    claim,
    identity::{LaunchIdentity, generation_bytes},
    network::Egress,
    prepared::PreparedGeneration,
};

/// One sandbox that reached Ready, and how it came to exist.
pub(super) struct Started {
    /// Whether a prepared machine served this Launch or it built its own.
    pub(super) preparation: PreparationClass,
    /// Whichever process now holds the machine.
    pub(super) held: Held,
    /// When this Launch finished producing a machine, in nanoseconds since it was accepted.
    pub(super) launched: u64,
    /// The head ranges the guest read before Ready, for the pool to warm the next head with.
    pub(super) head_reads: Vec<soma_kvm::head_reads::HeadSpan>,
}

/// Everything a Launch needs to turn a claim or a restore into a running Instance.
///
/// The network and the secrets are owned rather than borrowed because they are transferred into
/// exactly one machine and must not be readable anywhere afterwards.
pub(super) struct Launching<'a> {
    pub(super) instance: &'a InstanceId,
    pub(super) prepared: &'a PreparedGeneration,
    pub(super) identity: LaunchIdentity,
    pub(super) memory_mib: u64,
    pub(super) storage_mib: u64,
    pub(super) network: Network,
    pub(super) secrets: Vec<SecretFile>,
    /// This Instance's durable volume, absent when its Generation declares none.
    pub(super) volume: Option<soma::VolumeAttachment>,
}

pub(in crate::backend::kvm) const fn failure_kind(error: SessionError) -> BackendFailureKind {
    match error {
        // The machine could not be built from artifacts the host presented as prepared, which
        // is a property of the host rather than of the request.
        SessionError::Create | SessionError::LaunchPage | SessionError::Network => {
            BackendFailureKind::Unavailable
        }
        // The guest exists but never reached, or lost, its authenticated session.
        SessionError::Boot
        | SessionError::Ready
        | SessionError::Secret
        | SessionError::Execute
        | SessionError::File
        | SessionError::Pty
        | SessionError::Gone
        | SessionError::Poisoned => BackendFailureKind::GuestFailure,
    }
}

impl KvmBackend {
    /// Serves this Launch from a machine the pool prepared before the request arrived.
    ///
    /// The claimed machine is either assigned here or destroyed: `assign` consumes the claim,
    /// and a transfer that did not certainly complete never returns the machine to the pool.
    pub(super) fn assign_claimed(
        &mut self,
        operation: &OperationId,
        launching: Launching<'_>,
        egress: &mut Egress,
        claimed: claim::ClaimedMachine,
    ) -> Result<Started, BackendFailure> {
        let Launching {
            instance,
            prepared,
            identity,
            memory_mib: _,
            storage_mib: _,
            network,
            secrets,
            volume,
        } = launching;
        self.register(operation, instance, identity.guest_cid)?;
        // The machine already exists, so this stamp separates the fresh authority this Launch
        // transferred from the session it then drives, exactly as the on-demand arm does.
        let launched = self.clocks.elapsed_ns(operation);
        match claimed.assign(
            prepared,
            identity,
            network,
            secrets,
            volume,
            &mut |receipt| {
                egress
                    .activation(receipt)
                    .map_err(|()| SessionError::Network)
            },
        ) {
            Ok(session) => {
                // Read before the session is handed over: this is the only point at which the
                // launch's own view of the window is available.
                let head_reads = session.head_reads().to_vec();
                Ok(Started {
                    preparation: PreparationClass::PreparedWorker,
                    held: Held::Resident(session),
                    launched,
                    head_reads,
                })
            }
            Err(error) => Err(self.withdraw(operation, instance, error)),
        }
    }

    /// Serves this Launch by building the machine inside a jail this host holds nothing of.
    ///
    /// The Instance identity, the network claim, and the Host Runtime registration stay here,
    /// because they are the broker's work and none of them is something a jailed process can
    /// do. What crosses into the jail is a sealed descriptor table and nothing else.
    pub(super) fn launch_in_a_jail(
        &mut self,
        operation: &OperationId,
        launching: Launching<'_>,
    ) -> Result<Started, BackendFailure> {
        let Some(anchors) = self.jail.take() else {
            return Err(self.fail(operation, BackendFailureKind::Unsupported));
        };
        let outcome = self.build_in_a_jail(operation, launching, &anchors);
        self.jail = Some(anchors);
        outcome
    }

    fn build_in_a_jail(
        &mut self,
        operation: &OperationId,
        launching: Launching<'_>,
        anchors: &Anchors,
    ) -> Result<Started, BackendFailure> {
        let Launching {
            instance,
            prepared,
            identity,
            memory_mib,
            storage_mib,
            network,
            secrets,
            volume,
        } = launching;
        // A jailed machine is given no frame path and no secret, so a request that needs either
        // is refused rather than served by a machine that silently has neither.
        if network.attachment.is_some() || network.activation.is_some() || !secrets.is_empty() {
            return Err(self.fail(operation, BackendFailureKind::Unsupported));
        }
        // A jail's descriptor table carries the head and not a volume, so a Generation that
        // declares one is refused here rather than started with an empty disk.
        if volume.is_some() {
            return Err(self.fail(operation, BackendFailureKind::Unsupported));
        }
        // Only a Generation with a captured machine can be restored from descriptors; a cold
        // boot would need the kernel and initramfs this descriptor table has no roles for.
        if claim::snapshot(prepared).is_none() {
            return Err(self.fail(operation, BackendFailureKind::Unsupported));
        }
        let generation_bytes =
            generation_bytes(&prepared.id).map_err(|kind| self.fail(operation, kind))?;
        self.register(operation, instance, identity.guest_cid)?;
        let launched = self.clocks.elapsed_ns(operation);
        match Jailed::launch(
            anchors,
            &JailedLaunching {
                prepared,
                instance,
                instance_bytes: identity.instance,
                generation_bytes,
                memory_mib,
                disk_mib: storage_mib,
            },
        ) {
            Ok(jailed) => Ok(Started {
                preparation: PreparationClass::OnDemand,
                held: Held::Jailed(Box::new(jailed)),
                launched,
                // A jailed machine has no private head of its own to warm.
                head_reads: Vec::new(),
            }),
            Err(kind) => {
                // A launch that never produced a machine leaves no Instance owned by this Host.
                self.ownership.withdraw(instance);
                Err(BackendFailure::new(kind, self.clocks.elapsed_ns(operation)))
            }
        }
    }

    /// Serves this Launch by building its own machine, because none was prepared for it.
    pub(super) fn restore_on_demand(
        &mut self,
        operation: &OperationId,
        launching: Launching<'_>,
        egress: &mut Egress,
    ) -> Result<Started, BackendFailure> {
        let Launching {
            instance,
            prepared,
            identity,
            memory_mib,
            storage_mib: _,
            network,
            secrets,
            volume,
        } = launching;
        let boot = boot_for(prepared, memory_mib, identity, network, secrets, volume)
            .map_err(|kind| self.fail(operation, kind))?;
        self.register(operation, instance, boot.guest_cid)?;
        let launched = self.clocks.elapsed_ns(operation);
        match Session::launch(boot, &mut |receipt| {
            egress
                .activation(receipt)
                .map_err(|()| SessionError::Network)
        }) {
            Ok(session) => Ok(Started {
                preparation: PreparationClass::OnDemand,
                held: Held::Resident(session),
                launched,
                // A machine that built its own disk built it where the guest can reach it already.
                head_reads: Vec::new(),
            }),
            Err(error) => Err(self.withdraw(operation, instance, error)),
        }
    }

    /// Gives the Host Runtime, where one is configured, ownership of the Instance identity from
    /// before the machine exists, so a later process can address and end this Instance rather
    /// than finding nothing once this process is gone.
    fn register(
        &mut self,
        operation: &OperationId,
        instance: &InstanceId,
        guest_cid: u32,
    ) -> Result<(), BackendFailure> {
        self.ownership
            .register(instance, operation, guest_cid)
            .map_err(|kind| self.fail(operation, kind))
    }

    /// Ends the ownership of an Instance whose machine never reached Ready.
    ///
    /// A launch that ends here drops the lease, and dropping a lease releases it, so a guest
    /// that never reached its session leaves the broker holding nothing. The registration is
    /// withdrawn for the same reason: a Host owning an Instance no process serves is an
    /// Instance no client can ever end.
    fn withdraw(
        &mut self,
        operation: &OperationId,
        instance: &InstanceId,
        error: SessionError,
    ) -> BackendFailure {
        self.ownership.withdraw(instance);
        BackendFailure::new(failure_kind(error), self.clocks.elapsed_ns(operation))
    }
}
