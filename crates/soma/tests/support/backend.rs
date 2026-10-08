use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use soma::{
    Backend, BackendFailure, BackendFailureKind, BackendKind, CleanupEvidence, CleanupMethod,
    CleanupObservation, CleanupReason, CleanupTimes, CommandObservation, CommandStatus,
    CommandTimes, FileObservation, GenerationId, InstanceId, IsolationClass, LaunchObservation,
    LaunchTimes, OciDigest, OciPlatform, PreparationClass, PtyObservation, ResolutionObservation,
    SandboxLiveness, WorkloadIdentity,
};

use super::{CallGate, Mode, observed_network, terminal};

mod filesystem;
use filesystem::answer_for;

/// A flat guest filesystem shared with every clone of one backend.
type SharedFiles = Arc<Mutex<BTreeMap<Vec<u8>, Vec<u8>>>>;

#[derive(Clone)]
pub struct TestBackend {
    mode: Mode,
    calls: Arc<Mutex<Vec<&'static str>>>,
    execute_gate: Option<CallGate>,
    cleanup_gate: Option<CallGate>,
    /// A flat in-memory filesystem, so the engine's filesystem path can be exercised without KVM.
    files: SharedFiles,
    /// The one terminal session, for the same reason the filesystem is here.
    terminal: terminal::SharedTerminal,
    /// How many commands this machine has already run.
    ///
    /// A fixture mode models one command going wrong, not a machine that is broken. The first
    /// command takes the mode's outcome and every later one exits zero, so a test can ask what a
    /// sandbox does *after* a failure. A machine that was released answers the follow-up with a
    /// state failure instead, which is exactly the difference these tests read.
    executions: Arc<AtomicUsize>,
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
                files: Arc::new(Mutex::new(BTreeMap::new())),
                terminal: Arc::new(Mutex::new(None)),
                executions: Arc::new(AtomicUsize::new(0)),
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

    fn file(&mut self, request: soma::FileRequest<'_>) -> Result<FileObservation, BackendFailure> {
        self.record("file");
        let mut files = self.files.lock().expect("file map poisoned");
        let answer = answer_for(&mut files, request.operation());
        Ok(FileObservation::new(
            request.operation_id().clone(),
            request.instance_id().clone(),
            answer,
        ))
    }

    fn pty(&mut self, request: soma::PtyRequest<'_>) -> Result<PtyObservation, BackendFailure> {
        self.record("pty");
        let mut session = self.terminal.lock().expect("terminal poisoned");
        let answer = terminal::answer_for(&mut session, request.operation());
        Ok(PtyObservation::new(
            request.operation_id().clone(),
            request.instance_id().clone(),
            answer,
        ))
    }

    fn liveness(&mut self, _instance_id: &InstanceId) -> SandboxLiveness {
        self.record("liveness");
        SandboxLiveness::Live
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
        let first = self.executions.fetch_add(1, Ordering::SeqCst) == 0;
        let mode = if first { self.mode } else { Mode::Happy };
        if matches!(mode, Mode::CommandFailure | Mode::FailureTimeRegression) {
            let occurred_at = if mode == Mode::FailureTimeRegression {
                35
            } else {
                55
            };
            return Err(BackendFailure::new(
                BackendFailureKind::GuestFailure,
                occurred_at,
            ));
        }
        if mode == Mode::WorkloadRejected {
            return Err(BackendFailure::new(
                BackendFailureKind::WorkloadRejected,
                55,
            ));
        }
        let status = match mode {
            Mode::Timeout => CommandStatus::TimedOut,
            Mode::CombinedOutputOverflow | Mode::OutputLimit => CommandStatus::OutputLimitExceeded,
            Mode::Signaled => CommandStatus::Signaled { signal: None },
            Mode::SpawnFailed => CommandStatus::SpawnFailed { errno: 2 },
            _ => CommandStatus::Exited { code: 0 },
        };
        let instance_id = if mode == Mode::CommandIdentityMismatch {
            InstanceId::new("99999999999999999999999999999999").expect("valid fixture identity")
        } else {
            request.instance_id().clone()
        };
        let times = if mode == Mode::NonMonotonicCommand {
            CommandTimes::new(39, 60)
        } else {
            CommandTimes::new(50, 60)
        };
        let output = match mode {
            Mode::BinaryOutput => soma::ObservedOutput::new(vec![0, 0xff, b'\n'], 3, vec![0x80], 1),
            Mode::CombinedOutputOverflow => {
                soma::ObservedOutput::new(vec![b'a'; 8], 8, vec![b'b'; 8], 8)
            }
            // A program that never started produced nothing on either stream.
            Mode::SpawnFailed => soma::ObservedOutput::new(Vec::new(), 0, Vec::new(), 0),
            // Eight bytes were kept of the twelve the command wrote, which is what a command
            // stopped at a ten byte allowance looks like from the host.
            Mode::OutputLimit => soma::ObservedOutput::new(vec![b'a'; 8], 12, Vec::new(), 0),
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
