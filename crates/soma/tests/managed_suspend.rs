//! A managed suspension (ADR 0034): refused by a backend that cannot capture, which leaves the
//! Instance running and its record active; and completed by one that can, which ends the Instance
//! with an exact, replayable `Suspended` receipt.

mod support;

use std::path::Path;

use soma::{
    Backend, BackendFailure, BackendFailureKind, BackendKind, CleanupObservation, CleanupRequest,
    CommandObservation, DestroyMachineRequest, DirectCommand, Engine, ExecuteMachineRequest,
    ExecutionLimits, ExecutionRequest, FileObservation, FileRequest, InspectionObservation,
    InspectionRequest, InstanceId, LaunchMachineRequest, LaunchObservation, LaunchRequest,
    MachineShape, ManagedFailure, ManagedStateError, OciImage, OperationId, PtyObservation,
    PtyRequest, ResolutionObservation, ResolutionRequest, SandboxLiveness, SuspendMachineRequest,
    SuspendObservation, SuspendRequest, TerminalStatus,
};
use support::{Mode, TestBackend};

/// The test backend, answering a suspension as a capturing backend would.
struct Capturing(TestBackend, Vec<String>);

impl Backend for Capturing {
    type PreparedWorkload = ();

    fn kind(&self) -> BackendKind {
        self.0.kind()
    }
    fn resolve(
        &mut self,
        request: ResolutionRequest<'_>,
    ) -> Result<ResolutionObservation<()>, BackendFailure> {
        self.0.resolve(request)
    }
    fn launch(
        &mut self,
        request: LaunchRequest<'_, ()>,
    ) -> Result<LaunchObservation, BackendFailure> {
        self.0.launch(request)
    }
    fn execute(
        &mut self,
        request: ExecutionRequest<'_>,
    ) -> Result<CommandObservation, BackendFailure> {
        self.0.execute(request)
    }
    fn file(&mut self, request: FileRequest<'_>) -> Result<FileObservation, BackendFailure> {
        self.0.file(request)
    }
    fn pty(&mut self, request: PtyRequest<'_>) -> Result<PtyObservation, BackendFailure> {
        self.0.pty(request)
    }
    fn liveness(&mut self, instance_id: &InstanceId) -> SandboxLiveness {
        self.0.liveness(instance_id)
    }
    fn inspect(
        &mut self,
        request: InspectionRequest<'_>,
    ) -> Result<InspectionObservation, BackendFailure> {
        self.0.inspect(request)
    }
    fn cleanup(
        &mut self,
        request: CleanupRequest<'_>,
    ) -> Result<CleanupObservation, BackendFailure> {
        self.0.cleanup(request)
    }
    fn suspend(
        &mut self,
        request: SuspendRequest<'_>,
    ) -> Result<SuspendObservation, BackendFailure> {
        self.1.push(request.staging().display().to_string());
        Ok(SuspendObservation::Suspended)
    }
}

fn operation(digit: char) -> OperationId {
    OperationId::new(digit.to_string().repeat(32)).expect("operation")
}

fn launch<B: Backend<PreparedWorkload = ()>>(engine: &mut Engine<B>) -> InstanceId {
    let instance = InstanceId::new("22222222222222222222222222222222").expect("instance");
    engine
        .launch_machine(LaunchMachineRequest::new(
            operation('1'),
            instance.clone(),
            OciImage::parse("node:22").expect("image"),
            MachineShape::new(1, 1_024, 8_192).expect("shape"),
        ))
        .expect("launch");
    instance
}

fn suspend(instance: &InstanceId, digit: char) -> SuspendMachineRequest {
    SuspendMachineRequest::new(
        operation(digit),
        instance.clone(),
        Path::new("/staging").to_path_buf(),
    )
}

#[test]
fn a_backend_that_cannot_capture_refuses_and_the_instance_keeps_running() {
    let (backend, calls) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::new(backend);
    let instance = launch(&mut engine);

    let refused = engine
        .suspend_machine(&suspend(&instance, '5'))
        .expect_err("refused");
    assert_eq!(
        refused,
        ManagedFailure::Backend(BackendFailureKind::Unsupported)
    );
    // The record is active again: the Instance still runs commands and can be destroyed.
    engine
        .execute_machine(ExecuteMachineRequest::new(
            operation('6'),
            instance.clone(),
            DirectCommand::new("/usr/local/bin/node", ["--version"]).expect("command"),
            ExecutionLimits::new(30_000, 16 * 1024 * 1024).expect("limits"),
        ))
        .expect("still runs");
    let destroyed = engine
        .destroy_machine(DestroyMachineRequest::new(operation('7'), instance))
        .expect("destroy");
    assert_eq!(
        destroyed.receipt().terminal_status(),
        &TerminalStatus::Destroyed
    );
    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "execute", "cleanup"]
    );
}

#[test]
fn a_suspended_instance_ends_with_a_replayable_suspended_receipt() {
    let (backend, calls) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::new(Capturing(backend, Vec::new()));
    let instance = launch(&mut engine);

    let first = engine
        .suspend_machine(&suspend(&instance, '5'))
        .expect("suspend");
    let replay = engine
        .suspend_machine(&suspend(&instance, '5'))
        .expect("replay");
    assert_eq!(first, replay);
    assert_eq!(
        first.receipt().terminal_status(),
        &TerminalStatus::Suspended
    );
    // The Instance is over: a resume is a new Instance, so this one runs nothing more.
    let after = engine
        .execute_machine(ExecuteMachineRequest::new(
            operation('6'),
            instance,
            DirectCommand::new("/usr/local/bin/node", ["--version"]).expect("command"),
            ExecutionLimits::new(30_000, 16 * 1024 * 1024).expect("limits"),
        ))
        .expect_err("ended");
    assert_eq!(
        after,
        ManagedFailure::State(ManagedStateError::MachineStopped)
    );
    // Captured once, then cleaned up once; the replay touched nothing.
    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "cleanup"]
    );
}
