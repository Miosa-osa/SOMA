use std::path::PathBuf;

use soma::{
    BackendFailureKind, ManagedFailure, ManagedStateError, RunFailure, RunFailureKind,
    TerminalStatus,
};
use soma_local::{
    BackendSelection, LocalFailureKind, LocalRuntime, LocalRuntimeConfig, probe_backend,
};

use crate::{
    BackendTarget, CommandResult, CommandStatus, DoctorReport, DoctorStatus, ExecutionReceipt,
    InspectResult, MachineResult, MachineState, RuntimeFailure, RuntimeFailureKind, RuntimeRequest,
    RuntimeResponse, ToolRuntime,
};

#[derive(Clone, Debug, Default)]
pub struct LocalToolRuntime {
    runtime_path: Option<PathBuf>,
    state_root: Option<PathBuf>,
}

impl LocalToolRuntime {
    #[must_use]
    pub const fn new(runtime_path: Option<PathBuf>, state_root: Option<PathBuf>) -> Self {
        Self {
            runtime_path,
            state_root,
        }
    }

    fn invoke_blocking(self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeFailure> {
        if let RuntimeRequest::Doctor { backend } = request {
            return self.doctor(backend).map(RuntimeResponse::Doctor);
        }
        let backend = request_backend(&request);
        let config =
            LocalRuntimeConfig::discover(selection(backend), self.runtime_path, self.state_root)
                .map_err(map_local_failure)?;
        let mut runtime = LocalRuntime::open(config).map_err(map_local_failure)?;
        invoke_runtime(&mut runtime, request)
    }

    fn doctor(&self, backend: BackendTarget) -> Result<DoctorReport, RuntimeFailure> {
        match probe_backend(selection(backend), self.runtime_path.clone()) {
            Ok(probe) => Ok(DoctorReport {
                backend,
                status: DoctorStatus::ProbePassed,
                supported_target: true,
                runtime_ready: true,
                production_ready: probe.production_ready(),
            }),
            Err(failure) => Ok(DoctorReport {
                backend,
                status: if failure.kind() == LocalFailureKind::UnsupportedTarget {
                    DoctorStatus::Unsupported
                } else {
                    DoctorStatus::ProbeFailed
                },
                supported_target: failure.kind() != LocalFailureKind::UnsupportedTarget,
                runtime_ready: false,
                production_ready: false,
            }),
        }
    }
}

impl ToolRuntime for LocalToolRuntime {
    async fn invoke(&self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeFailure> {
        let runtime = self.clone();
        tokio::task::spawn_blocking(move || runtime.invoke_blocking(request))
            .await
            .map_err(|_| RuntimeFailure::new(RuntimeFailureKind::Internal))?
    }
}

fn invoke_runtime(
    runtime: &mut LocalRuntime,
    request: RuntimeRequest,
) -> Result<RuntimeResponse, RuntimeFailure> {
    match request {
        RuntimeRequest::Doctor { .. } => unreachable!("doctor is handled before runtime setup"),
        RuntimeRequest::Run(request) => {
            let instance = request.instance_id().clone();
            runtime
                .run(request.into_facade())
                .map(|outcome| {
                    command_result(instance, outcome.receipt(), outcome.output())
                        .map(RuntimeResponse::Run)
                })
                .map_err(|failure| map_run_failure(&failure))?
        }
        RuntimeRequest::Launch(request) => {
            let instance = request.instance_id().clone();
            runtime
                .launch_machine(request.into_facade())
                .map(|outcome| {
                    machine_result(instance, MachineState::Ready, outcome.receipt())
                        .map(RuntimeResponse::Launch)
                })
                .map_err(|failure| map_managed_failure(&failure))?
        }
        RuntimeRequest::Exec(request) => {
            let instance = request.instance_id().clone();
            runtime
                .execute_machine(request.into_facade())
                .map(|outcome| {
                    command_result(instance, outcome.receipt(), outcome.output())
                        .map(RuntimeResponse::Exec)
                })
                .map_err(|failure| map_managed_failure(&failure))?
        }
        RuntimeRequest::Inspect(request) => {
            let instance = request.instance_id().clone();
            let backend = request.backend();
            runtime
                .inspect_machine(request.into_facade())
                .map(|outcome| {
                    receipt(outcome.receipt()).map(|receipt| {
                        RuntimeResponse::Inspect(InspectResult::new(
                            instance,
                            machine_state(outcome.state()),
                            backend,
                            Some(receipt),
                        ))
                    })
                })
                .map_err(|failure| map_managed_failure(&failure))?
        }
        RuntimeRequest::Stop(request) => {
            let instance = request.instance_id().clone();
            runtime
                .stop_machine(request.into_facade())
                .map(|outcome| {
                    machine_result(instance, MachineState::Stopped, outcome.receipt())
                        .map(RuntimeResponse::Stop)
                })
                .map_err(|failure| map_managed_failure(&failure))?
        }
        RuntimeRequest::Destroy(request) => {
            let instance = request.instance_id().clone();
            runtime
                .destroy_machine(request.into_facade())
                .map(|outcome| {
                    machine_result(instance, MachineState::Destroyed, outcome.receipt())
                        .map(RuntimeResponse::Destroy)
                })
                .map_err(|failure| map_managed_failure(&failure))?
        }
    }
}

fn command_result(
    instance: soma::InstanceId,
    evidence: &soma::ExecutionReceipt,
    output: &soma::CapturedOutput,
) -> Result<CommandResult, RuntimeFailure> {
    let status = terminal_status(evidence.terminal_status())
        .ok_or_else(|| RuntimeFailure::new(RuntimeFailureKind::Internal))?;
    Ok(CommandResult::new(
        instance,
        status,
        output.stdout().to_vec(),
        output.stderr().to_vec(),
        receipt(evidence)?,
    ))
}

fn machine_result(
    instance: soma::InstanceId,
    state: MachineState,
    evidence: &soma::ExecutionReceipt,
) -> Result<MachineResult, RuntimeFailure> {
    Ok(MachineResult::new(instance, state, receipt(evidence)?))
}

fn receipt(evidence: &soma::ExecutionReceipt) -> Result<ExecutionReceipt, RuntimeFailure> {
    serde_json::to_value(evidence)
        .map_err(|_| RuntimeFailure::new(RuntimeFailureKind::Internal))
        .and_then(|value| {
            ExecutionReceipt::new(value)
                .map_err(|_| RuntimeFailure::new(RuntimeFailureKind::Internal))
        })
}

fn map_run_failure(failure: &RunFailure) -> RuntimeFailure {
    let kind = map_run_failure_kind(failure.kind());
    let Ok(receipt) = receipt(failure.receipt()) else {
        return RuntimeFailure::new(RuntimeFailureKind::Internal);
    };
    if let (Some(output), Some(status)) = (
        failure.output(),
        terminal_status(failure.receipt().terminal_status()),
    ) {
        RuntimeFailure::with_command_evidence(
            kind,
            receipt,
            status,
            output.stdout().to_vec(),
            output.stderr().to_vec(),
        )
    } else {
        RuntimeFailure::with_receipt(kind, receipt)
    }
}

fn map_managed_failure(failure: &ManagedFailure) -> RuntimeFailure {
    match failure {
        ManagedFailure::Operation(failure) => map_run_failure(failure),
        ManagedFailure::State(state) => RuntimeFailure::new(match state {
            ManagedStateError::MachineNotFound => RuntimeFailureKind::NotFound,
            ManagedStateError::MachineAlreadyExists
            | ManagedStateError::MachineStopped
            | ManagedStateError::OperationConflict
            | ManagedStateError::RecoveryRequired
            | ManagedStateError::ReplayCapacityReached => RuntimeFailureKind::Conflict,
        }),
        ManagedFailure::StateStore(_) => RuntimeFailure::new(RuntimeFailureKind::Internal),
        ManagedFailure::ReplayUnavailable(replay) => replay.receipt().map_or_else(
            || RuntimeFailure::new(RuntimeFailureKind::Conflict),
            |evidence| {
                receipt(evidence).map_or_else(
                    |_| RuntimeFailure::new(RuntimeFailureKind::Internal),
                    |receipt| RuntimeFailure::with_receipt(RuntimeFailureKind::Conflict, receipt),
                )
            },
        ),
    }
}

const fn map_run_failure_kind(kind: RunFailureKind) -> RuntimeFailureKind {
    match kind {
        RunFailureKind::Backend { kind, .. } => match kind {
            BackendFailureKind::Unsupported => RuntimeFailureKind::Unsupported,
            BackendFailureKind::Unavailable => RuntimeFailureKind::Unavailable,
            BackendFailureKind::ResourceConflict => RuntimeFailureKind::Conflict,
            BackendFailureKind::WorkloadRejected => RuntimeFailureKind::Rejected,
            BackendFailureKind::Timeout => RuntimeFailureKind::Timeout,
            BackendFailureKind::OutputLimit => RuntimeFailureKind::OutputLimit,
            BackendFailureKind::CleanupFailure => RuntimeFailureKind::CleanupIncomplete,
            BackendFailureKind::IsolationFailure | BackendFailureKind::GuestFailure => {
                RuntimeFailureKind::Internal
            }
        },
        RunFailureKind::TimedOut => RuntimeFailureKind::Timeout,
        RunFailureKind::OutputLimitExceeded => RuntimeFailureKind::OutputLimit,
        RunFailureKind::CleanupIncomplete => RuntimeFailureKind::CleanupIncomplete,
        RunFailureKind::Interrupted
        | RunFailureKind::ObservationMismatch
        | RunFailureKind::StateStore { .. } => RuntimeFailureKind::Internal,
    }
}

const fn terminal_status(status: &TerminalStatus) -> Option<CommandStatus> {
    match *status {
        TerminalStatus::Exited { code } => Some(CommandStatus::Exited { code }),
        TerminalStatus::Signaled { signal } => Some(CommandStatus::Signaled { signal }),
        TerminalStatus::TimedOut => Some(CommandStatus::TimedOut),
        TerminalStatus::OutputLimitExceeded => Some(CommandStatus::OutputLimitExceeded),
        _ => None,
    }
}

const fn machine_state(state: soma::MachineState) -> MachineState {
    match state {
        soma::MachineState::Ready => MachineState::Ready,
        soma::MachineState::Stopping => MachineState::Stopping,
    }
}

const fn request_backend(request: &RuntimeRequest) -> BackendTarget {
    match request {
        RuntimeRequest::Doctor { backend } => *backend,
        RuntimeRequest::Run(request) => request.backend(),
        RuntimeRequest::Launch(request) => request.backend(),
        RuntimeRequest::Exec(request) => request.backend(),
        RuntimeRequest::Inspect(request) => request.backend(),
        RuntimeRequest::Stop(request) => request.backend(),
        RuntimeRequest::Destroy(request) => request.backend(),
    }
}

const fn selection(target: BackendTarget) -> BackendSelection {
    match target {
        BackendTarget::Auto | BackendTarget::Local => BackendSelection::Auto,
        BackendTarget::Kvm => BackendSelection::Kvm,
        BackendTarget::Macos => BackendSelection::Macos,
    }
}

const fn map_local_failure(failure: soma_local::LocalFailure) -> RuntimeFailure {
    RuntimeFailure::new(match failure.kind() {
        LocalFailureKind::InvalidConfiguration => RuntimeFailureKind::Rejected,
        LocalFailureKind::UnsupportedTarget => RuntimeFailureKind::Unsupported,
        LocalFailureKind::BackendUnavailable => RuntimeFailureKind::Unavailable,
        LocalFailureKind::StateStore(_) => RuntimeFailureKind::Internal,
    })
}
