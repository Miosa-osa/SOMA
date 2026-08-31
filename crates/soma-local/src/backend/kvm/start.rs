//! How one Launch comes to hold a machine, either way it gets one.
//!
//! A Launch either claims a machine the pool prepared before the request arrived or restores one
//! of its own. Everything after that point is identical: the same registration with the Host
//! Runtime, the same fresh authority, the same mint, broker activation and link gate, and the
//! same withdrawal when the machine never reaches Ready. The arms differ only in where the
//! machine came from, which is exactly what the reported preparation class names.

use soma::{BackendFailure, BackendFailureKind, InstanceId, OperationId, PreparationClass};
use soma_guest::SecretFile;

use super::{
    KvmBackend,
    boot::boot_for,
    claim,
    identity::LaunchIdentity,
    network::Egress,
    prepared::PreparedGeneration,
    session::{Network, Session, SessionError},
};

/// One sandbox that reached Ready, and how it came to exist.
pub(super) struct Started {
    /// Whether a prepared machine served this Launch or it built its own.
    pub(super) preparation: PreparationClass,
    /// The authenticated session over the machine.
    pub(super) session: Session,
    /// When this Launch finished producing a machine, in nanoseconds since it was accepted.
    pub(super) launched: u64,
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
    pub(super) network: Network,
    pub(super) secrets: Vec<SecretFile>,
}

pub(super) const fn failure_kind(error: SessionError) -> BackendFailureKind {
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
            network,
            secrets,
        } = launching;
        let assignment =
            claim::assignment_for(&claimed.snapshot, prepared, identity, network, secrets)
                .map_err(|kind| self.fail(operation, kind))?;
        self.register(operation, instance, identity.guest_cid)?;
        // The machine already exists, so this stamp separates the fresh authority this Launch
        // transferred from the session it then drives, exactly as the on-demand arm does.
        let launched = self.clocks.elapsed_ns(operation);
        match claimed.machine.assign(assignment, &mut |receipt| {
            egress.activate(receipt).map_err(|()| SessionError::Network)
        }) {
            Ok(session) => Ok(Started {
                preparation: PreparationClass::PreparedWorker,
                session,
                launched,
            }),
            Err(error) => Err(self.withdraw(operation, instance, error)),
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
            network,
            secrets,
        } = launching;
        let boot = boot_for(prepared, memory_mib, identity, network, secrets)
            .map_err(|kind| self.fail(operation, kind))?;
        self.register(operation, instance, boot.guest_cid)?;
        let launched = self.clocks.elapsed_ns(operation);
        match Session::launch(boot, &mut |receipt| {
            egress.activate(receipt).map_err(|()| SessionError::Network)
        }) {
            Ok(session) => Ok(Started {
                preparation: PreparationClass::OnDemand,
                session,
                launched,
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
