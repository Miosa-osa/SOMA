use std::path::PathBuf;

use soma::TerminalStatus;
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
            return Ok(RuntimeResponse::Doctor(self.doctor(backend)));
        }
        let backend = request_backend(&request);
        // Every managed Machine tool names an Instance a later call must be able to reach, and
        // each call opens its own runtime, so the machine has to be held by a host rather than by
        // whichever call happened to create it. `soma_run` needs no host: it launches, runs, and
        // releases inside one call.
        let hosted = !matches!(request, RuntimeRequest::Run(_));
        let config =
            LocalRuntimeConfig::discover(selection(backend), self.runtime_path, self.state_root)
                .map_err(map_local_failure)?
                .with_hosted_machines(hosted);
        let mut runtime = LocalRuntime::open(config).map_err(map_local_failure)?;
        invoke_runtime(&mut runtime, request)
    }

    fn doctor(&self, backend: BackendTarget) -> DoctorReport {
        match probe_backend(selection(backend), self.runtime_path.clone()) {
            Ok(probe) => DoctorReport {
                backend,
                status: DoctorStatus::ProbePassed,
                supported_target: true,
                runtime_ready: true,
                production_ready: probe.production_ready(),
            },
            Err(failure) => DoctorReport {
                backend,
                status: if failure.kind() == LocalFailureKind::UnsupportedTarget {
                    DoctorStatus::Unsupported
                } else {
                    DoctorStatus::ProbeFailed
                },
                supported_target: failure.kind() != LocalFailureKind::UnsupportedTarget,
                runtime_ready: false,
                production_ready: false,
            },
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

mod failure;

use self::failure::{map_managed_failure, map_run_failure};

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
    let status = terminal_status(*evidence.terminal_status())
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

pub(super) fn receipt(
    evidence: &soma::ExecutionReceipt,
) -> Result<ExecutionReceipt, RuntimeFailure> {
    serde_json::to_value(evidence)
        .map_err(|_| RuntimeFailure::new(RuntimeFailureKind::Internal))
        .and_then(|value| {
            ExecutionReceipt::new(value)
                .map_err(|_| RuntimeFailure::new(RuntimeFailureKind::Internal))
        })
}

pub(super) const fn terminal_status(status: TerminalStatus) -> Option<CommandStatus> {
    match status {
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
        BackendTarget::Docker => BackendSelection::Docker,
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
