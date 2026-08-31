//! Launch, execute, inspect, and cleanup for one KVM sandbox.
//!
//! The Backend holds at most one live sandbox at a time, keyed by the Instance that launched it,
//! because one command-line invocation drives one sandbox from launch to cleanup. A request that
//! names a different Instance is refused rather than served by the wrong machine.
//!
//! Ownership of the Instance identity is a separate question from residency of the session.
//! Where a Host Runtime is configured, it owns the identity across processes, so Launch
//! registers with it and Cleanup ends that ownership by identity. The guest session is still
//! resident in the process that launched it, so an Execute from a second process is refused
//! with a typed unsupported rather than served or reported absent; that stays true until the
//! machine runs in the worker the Host owns rather than inside the launching process.

use std::time::Duration;

use soma::{
    BackendFailure, BackendFailureKind, CleanupDisposition, CleanupEvidence, CleanupMethod,
    CleanupObservation, CleanupRequest, CleanupTimes, CommandObservation, CommandTimes,
    DigestBinding, ExecutionRequest, InspectionObservation, InspectionRequest, InstanceId,
    IsolationClass, LaunchObservation, LaunchRequest, LaunchTimes, MachineState, PreparationClass,
};

use super::{
    KvmBackend,
    boot::boot_for,
    evidence::{CONTRACT_VCPUS, effective_network, effective_shape, guest_command, observation},
    identity::LaunchIdentity,
    network::{Egress, Released},
    session::{Network, Session, SessionError},
};

/// How long one bounded command may take before the session is considered gone.
pub(in crate::backend::kvm) const COMMAND_CEILING: Duration = Duration::from_secs(300);

/// The live sandbox this Backend is driving.
pub(super) struct Live {
    pub(super) instance: InstanceId,
    pub(super) session: Session,
    /// The network this Instance holds, released when its sandbox is.
    pub(super) egress: Egress,
    /// What this Instance was told its network is, reported again by every later observation.
    pub(super) network: soma::EffectiveNetwork,
}

const fn failure_kind(error: SessionError) -> BackendFailureKind {
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
    pub(in crate::backend) fn launch(
        &mut self,
        request: &LaunchRequest<'_, Box<dyn std::any::Any + Send>>,
    ) -> Result<LaunchObservation, BackendFailure> {
        let operation = request.operation_id();
        let admitted = self.clocks.elapsed_ns(operation);
        // This Backend owns at most one sandbox. Assigning over a live one would drop its
        // Session, and dropping a Session shuts the guest down, so launching B would silently
        // destroy A without any Stop or Cleanup naming A.
        if self.live.is_some() {
            return Err(self.fail(operation, BackendFailureKind::ResourceConflict));
        }
        let shape = request.shape();
        // The machine contract fixes one vCPU, so a larger shape is refused rather than
        // silently served by a machine that is not the shape the caller asked for.
        if shape.vcpu_count() != CONTRACT_VCPUS {
            return Err(self.fail(operation, BackendFailureKind::WorkloadRejected));
        }
        let prepared = request
            .prepared()
            .downcast_ref::<super::prepared::PreparedGeneration>()
            .ok_or_else(|| self.fail(operation, BackendFailureKind::WorkloadRejected))?;
        let identity = LaunchIdentity::derive(request.instance_id())
            .map_err(|kind| self.fail(operation, kind))?;
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
        // No secret reaches this Backend yet. The portable Launch request carries a Template's
        // secret references, not their values, and the host side that resolves a reference into
        // a value is the credential mediator of the second delivery mode, which does not exist.
        // The placement itself is wired, so a launch given a secret it cannot place fails here
        // rather than running without it.
        let secrets = Vec::new();
        let boot = boot_for(prepared, shape.memory_mib(), identity, network, secrets)
            .map_err(|kind| self.fail(operation, kind))?;
        // The Host Runtime, where one is configured, owns the Instance identity from before the
        // machine exists, so a later process can address and end this Instance rather than
        // finding nothing once this process is gone.
        let registered = self
            .ownership
            .register(request.instance_id(), operation, boot.guest_cid);
        registered.map_err(|kind| self.fail(operation, kind))?;
        let launched = self.clocks.elapsed_ns(operation);
        // A launch that ends here drops the lease, and dropping a lease releases it, so a guest
        // that never reached its session leaves the broker holding nothing. The registration is
        // withdrawn for the same reason: a Host owning an Instance no process serves is an
        // Instance no client can ever end.
        let session = match Session::launch(boot, &mut |receipt| {
            egress.activate(receipt).map_err(|()| SessionError::Network)
        }) {
            Ok(session) => session,
            Err(error) => {
                self.ownership.withdraw(request.instance_id());
                return Err(BackendFailure::new(
                    failure_kind(error),
                    self.clocks.elapsed_ns(operation),
                ));
            }
        };
        let ready = self.clocks.elapsed_ns(operation);
        self.live = Some(Live {
            instance: request.instance_id().clone(),
            session,
            egress,
            network: observed.clone(),
        });
        Ok(LaunchObservation::new(
            operation.clone(),
            request.instance_id().clone(),
            request.workload().clone(),
            Self::kind(),
            IsolationClass::HardwareVirtualMachine,
            // Every launch cold boots its own machine; no worker was prepared for it.
            PreparationClass::OnDemand,
            // What a host prepares today is a Candidate, and no certification gate has
            // verified it, so the artifacts are observed rather than enforced. Reporting
            // LaunchEnforced would claim a binding no gate produced. It becomes enforced when
            // Launch accepts only a certified Generation.
            DigestBinding::ObservedOnly,
            effective_shape(shape.memory_mib()),
            observed,
            LaunchTimes::new(admitted, launched, ready),
        ))
    }

    pub(in crate::backend) fn execute(
        &mut self,
        request: ExecutionRequest<'_>,
    ) -> Result<CommandObservation, BackendFailure> {
        let operation = request.operation_id();
        let started = self.clocks.elapsed_ns(operation);
        let command = guest_command(&request)
            .ok_or_else(|| self.fail(operation, BackendFailureKind::WorkloadRejected))?;
        let Some(live) = self.live_for(request.instance_id()) else {
            let kind = self.absent_kind(request.instance_id());
            return Err(self.fail(operation, kind));
        };
        let completed = live
            .session
            .execute(command, COMMAND_CEILING)
            .map_err(|error| {
                BackendFailure::new(failure_kind(error), self.clocks.elapsed_ns(operation))
            })?;
        let finished = self.clocks.elapsed_ns(operation);
        observation(&request, &completed, CommandTimes::new(started, finished))
            .ok_or_else(|| BackendFailure::new(BackendFailureKind::GuestFailure, finished))
    }

    pub(in crate::backend) fn inspect(
        &mut self,
        request: InspectionRequest<'_>,
    ) -> Result<InspectionObservation, BackendFailure> {
        let operation = request.operation_id();
        let observed = self.clocks.elapsed_ns(operation);
        // A sandbox this Backend is not driving cannot be observed, and reporting it as stopped
        // would claim knowledge of a machine this process never owned.
        let Some(live) = self.live_for(request.instance_id()) else {
            let kind = self.absent_kind(request.instance_id());
            return Err(self.fail(operation, kind));
        };
        let network = live.network.clone();
        Ok(InspectionObservation::observed(
            request,
            Self::kind(),
            MachineState::Ready,
            network,
            observed,
        ))
    }

    pub(in crate::backend) fn cleanup(
        &mut self,
        request: CleanupRequest<'_>,
    ) -> Result<CleanupObservation, BackendFailure> {
        let operation = request.operation_id();
        let started = self.clocks.elapsed_ns(operation);
        // Cleanup is idempotent, but an Instance this Backend never owned is a different fact
        // from one it owned and released. Reporting resources complete for an unknown Instance
        // would claim to have released resources this process never held.
        let Some(mut live) = self.take_live(request.instance_id()) else {
            // With no session here the Instance identity may still be owned by the Host
            // Runtime, and ending that ownership is the one terminal act this process can
            // perform for it. The dispositions stay NotOwned because that is what happened:
            // this process held no machine, no memory, no head, and no guest authority.
            let ended = self.ownership.release(request.instance_id());
            ended.map_err(|kind| self.fail(operation, kind))?;
            let finished = self.clocks.elapsed_ns(operation);
            return Ok(CleanupObservation::new(
                operation.clone(),
                request.instance_id().clone(),
                not_owned_evidence(),
                CleanupTimes::new(started, finished),
            ));
        };
        let outcome = live.session.shutdown();
        if let Ok(evidence) = &outcome {
            super::timeline::dump(request.instance_id().as_str(), evidence);
        }
        // The lease is released whether or not the guest shut down cleanly. A machine that is
        // gone has no use for a namespace, a TAP, an address lease, or a port mapping, and
        // leaving them behind is the failure that compounds fastest across many sandboxes.
        let network = match live.egress.release() {
            Released::Complete => CleanupDisposition::Complete,
            Released::NothingHeld => CleanupDisposition::NotOwned,
            // The broker was asked and could not confirm, so this process must not claim it did.
            Released::Incomplete => CleanupDisposition::Incomplete,
        };
        let released = outcome.is_ok();
        if !released {
            let finished = self.clocks.elapsed_ns(operation);
            return Err(BackendFailure::new(
                BackendFailureKind::CleanupFailure,
                finished,
            ));
        }
        // The machine is gone, so the Host must stop owning the Instance that named it. A
        // record left behind is a leak only reconciliation could find, and an ownership the
        // Host could not prove ended must not be reported as a complete cleanup.
        let ended = self.ownership.release(request.instance_id());
        let proven = ended.map_err(|kind| self.fail(operation, kind))?;
        let finished = self.clocks.elapsed_ns(operation);
        if !proven {
            return Err(BackendFailure::new(
                BackendFailureKind::CleanupFailure,
                finished,
            ));
        }
        // The machine, its memory mapping, the private overlay head, and the Instance authority
        // are all owned by the sandbox thread and released when it ends.
        let evidence = CleanupEvidence::new(
            CleanupDisposition::Complete,
            CleanupDisposition::Complete,
            CleanupDisposition::Complete,
            network,
            CleanupDisposition::Complete,
        )
        .with_method(CleanupMethod::Graceful);
        Ok(CleanupObservation::new(
            operation.clone(),
            request.instance_id().clone(),
            evidence,
            CleanupTimes::new(started, finished),
        ))
    }
}

#[path = "lifecycle/lookup.rs"]
mod lookup;

use lookup::not_owned_evidence;
