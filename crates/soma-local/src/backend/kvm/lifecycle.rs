//! Launch, execute, inspect, and cleanup for one KVM sandbox.
//!
//! The Backend holds at most one live sandbox at a time, keyed by the Instance that launched it,
//! because one command-line invocation drives one sandbox from launch to cleanup. A request that
//! names a different Instance is refused rather than served by the wrong machine.

use std::time::Duration;

use soma::{
    BackendFailure, BackendFailureKind, CleanupDisposition, CleanupEvidence, CleanupMethod,
    CleanupObservation, CleanupRequest, CleanupTimes, CommandObservation, CommandTimes,
    DigestBinding, EgressPolicy, ExecutionRequest, InspectionObservation, InspectionRequest,
    InstanceId, IsolationClass, LaunchObservation, LaunchRequest, LaunchTimes, MachineState,
    PreparationClass,
};

use super::{
    KvmBackend,
    boot::boot_for,
    evidence::{CONTRACT_VCPUS, effective_network, effective_shape, guest_command, observation},
    session::{Session, SessionError},
};

/// How long one bounded command may take before the session is considered gone.
const COMMAND_CEILING: Duration = Duration::from_secs(300);

/// The live sandbox this Backend is driving.
pub(super) struct Live {
    pub(super) instance: InstanceId,
    pub(super) session: Session,
}

const fn failure_kind(error: SessionError) -> BackendFailureKind {
    match error {
        // The machine could not be built from artifacts the host presented as prepared, which
        // is a property of the host rather than of the request.
        SessionError::Create | SessionError::LaunchPage => BackendFailureKind::Unavailable,
        // The guest exists but never reached, or lost, its authenticated session.
        SessionError::Boot | SessionError::Ready | SessionError::Execute | SessionError::Gone => {
            BackendFailureKind::GuestFailure
        }
    }
}

impl KvmBackend {
    pub(in crate::backend) fn launch(
        &mut self,
        request: &LaunchRequest<'_, Box<dyn std::any::Any + Send>>,
    ) -> Result<LaunchObservation, BackendFailure> {
        let operation = request.operation_id();
        let admitted = self.clocks.elapsed_ns(operation);
        let shape = request.shape();
        // The machine contract fixes one vCPU, so a larger shape is refused rather than
        // silently served by a machine that is not the shape the caller asked for.
        if shape.vcpu_count() != CONTRACT_VCPUS {
            return Err(self.fail(operation, BackendFailureKind::WorkloadRejected));
        }
        // The guest's one network device is link down today, so a request that needs egress
        // cannot be served. Saying so is the whole point of a fail-closed network.
        if !matches!(
            shape.capabilities().network_policy().egress(),
            EgressPolicy::Denied | EgressPolicy::Unspecified
        ) {
            return Err(self.fail(operation, BackendFailureKind::Unsupported));
        }
        let prepared = request
            .prepared()
            .downcast_ref::<super::prepared::PreparedGeneration>()
            .ok_or_else(|| self.fail(operation, BackendFailureKind::WorkloadRejected))?;
        let boot = boot_for(prepared, shape.memory_mib(), request.instance_id())
            .map_err(|kind| self.fail(operation, kind))?;
        let launched = self.clocks.elapsed_ns(operation);
        let session = Session::launch(boot).map_err(|error| {
            BackendFailure::new(failure_kind(error), self.clocks.elapsed_ns(operation))
        })?;
        let ready = self.clocks.elapsed_ns(operation);
        self.live = Some(Live {
            instance: request.instance_id().clone(),
            session,
        });
        Ok(LaunchObservation::new(
            operation.clone(),
            request.instance_id().clone(),
            request.workload().clone(),
            Self::kind(),
            IsolationClass::HardwareVirtualMachine,
            // Every launch cold boots its own machine; no worker was prepared for it.
            PreparationClass::OnDemand,
            // The Generation is selected by the exact bytes the host prepared, and the machine
            // is built from those artifacts, so the digest is enforced at launch.
            DigestBinding::LaunchEnforced,
            effective_shape(shape.memory_mib()),
            effective_network(),
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
            return Err(self.fail(operation, BackendFailureKind::Unavailable));
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
        let state = if self.live_for(request.instance_id()).is_some() {
            MachineState::Ready
        } else {
            return Err(self.fail(operation, BackendFailureKind::Unavailable));
        };
        Ok(InspectionObservation::observed(
            request,
            Self::kind(),
            state,
            effective_network(),
            observed,
        ))
    }

    pub(in crate::backend) fn cleanup(
        &mut self,
        request: CleanupRequest<'_>,
    ) -> Result<CleanupObservation, BackendFailure> {
        let operation = request.operation_id();
        let started = self.clocks.elapsed_ns(operation);
        // Cleanup is idempotent: a sandbox this Backend never held, or already released, is
        // already in the state the caller asked for.
        let released = match self.take_live(request.instance_id()) {
            Some(live) => live.session.shutdown().is_ok(),
            None => true,
        };
        let finished = self.clocks.elapsed_ns(operation);
        if !released {
            return Err(BackendFailure::new(
                BackendFailureKind::CleanupFailure,
                finished,
            ));
        }
        // The machine, its memory mapping, the private overlay head, and the Instance authority
        // are all owned by the sandbox thread and released when it ends. The network is not
        // owned at all, because the device is link down and no host resource backs it.
        let evidence = CleanupEvidence::new(
            CleanupDisposition::Complete,
            CleanupDisposition::Complete,
            CleanupDisposition::Complete,
            CleanupDisposition::NotOwned,
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

    fn fail(&mut self, operation: &soma::OperationId, kind: BackendFailureKind) -> BackendFailure {
        BackendFailure::new(kind, self.clocks.elapsed_ns(operation))
    }

    fn live_for(&mut self, instance: &InstanceId) -> Option<&mut Live> {
        self.live.as_mut().filter(|live| &live.instance == instance)
    }

    fn take_live(&mut self, instance: &InstanceId) -> Option<Live> {
        if self
            .live
            .as_ref()
            .is_some_and(|live| &live.instance == instance)
        {
            self.live.take()
        } else {
            None
        }
    }
}
