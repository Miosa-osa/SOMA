use std::sync::{Arc, Mutex};

use soma::{
    Backend, BackendFailure, BackendFailureKind, BackendKind, CleanupEvidence, CleanupMethod,
    CleanupObservation, CleanupReason, CleanupTimes, CommandObservation, CommandStatus,
    CommandTimes, GenerationId, InstanceId, IsolationClass, LaunchObservation, LaunchTimes,
    OciDigest, OciPlatform, PreparationClass, ResolutionObservation, WorkloadIdentity,
};

use super::{CallGate, Mode, observed_network};

#[derive(Clone)]
pub struct TestBackend {
    mode: Mode,
    calls: Arc<Mutex<Vec<&'static str>>>,
    execute_gate: Option<CallGate>,
    cleanup_gate: Option<CallGate>,
}

#[allow(
    dead_code,
    reason = "shared gated backend variants are used by selected integration tests"
)]
impl TestBackend {
    pub fn new(mode: Mode) -> (Self, Arc<Mutex<Vec<&'static str>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                mode,
                calls: Arc::clone(&calls),
                execute_gate: None,
                cleanup_gate: None,
            },
            calls,
        )
    }

    pub fn with_execute_gate(mut self) -> (Self, CallGate) {
        let gate = CallGate::new();
        self.execute_gate = Some(gate.clone());
        (self, gate)
    }

    pub fn with_cleanup_gate(mut self) -> (Self, CallGate) {
        let gate = CallGate::new();
        self.cleanup_gate = Some(gate.clone());
        (self, gate)
    }

    fn record(&self, name: &'static str) {
        self.calls.lock().expect("call log poisoned").push(name);
    }
}

impl Backend for TestBackend {
    type PreparedWorkload = ();

    fn kind(&self) -> BackendKind {
        BackendKind::MacosVirtualization
    }

    fn resolve(
        &mut self,
        request: soma::ResolutionRequest<'_>,
    ) -> Result<ResolutionObservation<Self::PreparedWorkload>, BackendFailure> {
        self.record("resolve");
        Ok(ResolutionObservation::new(
            request.operation_id().clone(),
            request.source_fingerprint().clone(),
            workload(),
            (),
            10,
        ))
    }

    fn launch(
        &mut self,
        request: soma::LaunchRequest<'_, Self::PreparedWorkload>,
    ) -> Result<LaunchObservation, BackendFailure> {
        self.record("launch");
        if self.mode == Mode::LaunchFailure {
            return Err(BackendFailure::new(
                BackendFailureKind::IsolationFailure,
                25,
            ));
        }
        let effective_shape = soma::EffectiveShape::fully_observed(request.shape());
        let effective_network = if self.mode == Mode::UnverifiedNetworkDenial {
            soma::EffectiveNetwork::unavailable(soma::ObservationUnavailable::NotVerified)
        } else {
            observed_network(request.shape().capabilities().network_policy())
        };
        Ok(LaunchObservation::new(
            request.operation_id().clone(),
            request.instance_id().clone(),
            request.workload().clone(),
            BackendKind::MacosVirtualization,
            IsolationClass::HardwareVirtualMachine,
            PreparationClass::OnDemand,
            soma::DigestBinding::ObservedOnly,
            effective_shape,
            effective_network,
            LaunchTimes::new(20, 30, 40),
        ))
    }

    fn execute(
        &mut self,
        request: soma::ExecutionRequest<'_>,
    ) -> Result<CommandObservation, BackendFailure> {
        self.record("execute");
        if let Some(gate) = &self.execute_gate {
            gate.block_backend();
        }
        if matches!(
            self.mode,
            Mode::CommandFailure | Mode::FailureTimeRegression
        ) {
            let occurred_at = if self.mode == Mode::FailureTimeRegression {
                35
            } else {
                55
            };
            return Err(BackendFailure::new(
                BackendFailureKind::GuestFailure,
                occurred_at,
            ));
        }
        let status = match self.mode {
            Mode::Timeout => CommandStatus::TimedOut,
            Mode::CombinedOutputOverflow => CommandStatus::OutputLimitExceeded,
            Mode::Signaled => CommandStatus::Signaled { signal: None },
            _ => CommandStatus::Exited { code: 0 },
        };
        let instance_id = if self.mode == Mode::CommandIdentityMismatch {
            InstanceId::new("99999999999999999999999999999999").expect("valid fixture identity")
        } else {
            request.instance_id().clone()
        };
        let times = if self.mode == Mode::NonMonotonicCommand {
            CommandTimes::new(39, 60)
        } else {
            CommandTimes::new(50, 60)
        };
        let output = match self.mode {
            Mode::BinaryOutput => soma::ObservedOutput::new(vec![0, 0xff, b'\n'], 3, vec![0x80], 1),
            Mode::CombinedOutputOverflow => {
                soma::ObservedOutput::new(vec![b'a'; 8], 8, vec![b'b'; 8], 8)
            }
            _ => soma::ObservedOutput::new(b"v22.23.2\n".to_vec(), 10, Vec::new(), 0),
        };
        Ok(CommandObservation::new(
            request.operation_id().clone(),
            instance_id,
            status,
            output,
            times,
        ))
    }

    fn inspect(
        &mut self,
        request: soma::InspectionRequest<'_>,
    ) -> Result<soma::InspectionObservation, BackendFailure> {
        self.record("inspect");
        Ok(soma::InspectionObservation::observed(
            request,
            BackendKind::MacosVirtualization,
            soma::MachineState::Ready,
            observed_network(request.shape().capabilities().network_policy()),
            15,
        ))
    }

    fn cleanup(
        &mut self,
        request: soma::CleanupRequest<'_>,
    ) -> Result<CleanupObservation, BackendFailure> {
        self.record("cleanup");
        if let Some(gate) = &self.cleanup_gate {
            gate.block_backend();
        }
        if matches!(
            self.mode,
            Mode::CleanupFailure | Mode::CleanupFailureTimeRegression
        ) {
            let occurred_at = if self.mode == Mode::CleanupFailureTimeRegression {
                35
            } else {
                75
            };
            return Err(BackendFailure::new(
                BackendFailureKind::CleanupFailure,
                occurred_at,
            ));
        }
        let method = match request.reason() {
            CleanupReason::GracefulStop if self.mode == Mode::GracefulFallback => {
                CleanupMethod::GracefulThenForced
            }
            CleanupReason::GracefulStop => CleanupMethod::Graceful,
            CleanupReason::RunCompleted
            | CleanupReason::Rollback
            | CleanupReason::ForcedDestroy
            | CleanupReason::UncertainCommandTermination => CleanupMethod::Forced,
        };
        Ok(CleanupObservation::new(
            request.operation_id().clone(),
            request.instance_id().clone(),
            CleanupEvidence::complete_owned_machine().with_method(method),
            CleanupTimes::new(70, 80),
        ))
    }
}

fn workload() -> WorkloadIdentity {
    WorkloadIdentity::new(
        OciDigest::parse("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .expect("valid digest"),
        OciPlatform::linux_arm64(),
        Some(GenerationId::new(format!("sha256:{}", "3".repeat(64))).expect("valid generation")),
    )
}
