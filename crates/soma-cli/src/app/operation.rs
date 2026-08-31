use std::path::PathBuf;

use soma_local::{LocalRuntime, LocalRuntimeConfig};

use crate::{
    cli::BackendSelection,
    exit::ProcessExit,
    model::{InspectionReport, Response, ResultBody},
    request::PreparedOperation,
};

use super::{
    Execution,
    failure::{local_failure, managed_failure, run_failure},
    success::{command_success, machine_success},
};

pub(super) fn invoke(
    backend: BackendSelection,
    runtime_path: Option<PathBuf>,
    state_root: Option<PathBuf>,
    operation: PreparedOperation,
) -> Execution {
    let command = operation.command();
    // Every managed Machine operation names an Instance a later process must be able to reach,
    // so its machine is held by a host rather than by this command. A run holds its own machine
    // for the whole operation and releases it before returning, so it needs no second process.
    let hosted = !matches!(operation, PreparedOperation::Run { .. });
    let config = match LocalRuntimeConfig::discover(backend.into(), runtime_path, state_root) {
        Ok(config) => config.with_hosted_machines(hosted),
        Err(failure) => return local_failure(command, failure.kind()),
    };
    let mut runtime = match LocalRuntime::open(config) {
        Ok(runtime) => runtime,
        Err(failure) => return local_failure(command, failure.kind()),
    };
    dispatch(&mut runtime, command, operation)
}

fn dispatch(
    runtime: &mut LocalRuntime,
    command: &'static str,
    operation: PreparedOperation,
) -> Execution {
    match operation {
        PreparedOperation::Run {
            instance_id,
            request,
        } => match runtime.run(request) {
            Ok(outcome) => {
                command_success(command, instance_id, outcome.receipt(), outcome.output())
            }
            Err(failure) => run_failure(command, instance_id, &failure),
        },
        PreparedOperation::Launch {
            instance_id,
            request,
        } => match runtime.launch_machine(request) {
            Ok(outcome) => machine_success(command, instance_id, "ready", outcome.receipt()),
            Err(failure) => managed_failure(command, instance_id, &failure),
        },
        PreparedOperation::Execute {
            instance_id,
            request,
        } => match runtime.execute_machine(request) {
            Ok(outcome) => {
                command_success(command, instance_id, outcome.receipt(), outcome.output())
            }
            Err(failure) => managed_failure(command, instance_id, &failure),
        },
        PreparedOperation::Inspect {
            instance_id,
            request,
        } => match runtime.inspect_machine(request) {
            Ok(outcome) => inspection_success(command, instance_id, &outcome),
            Err(failure) => managed_failure(command, instance_id, &failure),
        },
        PreparedOperation::Stop {
            instance_id,
            request,
        } => match runtime.stop_machine(request) {
            Ok(outcome) => machine_success(command, instance_id, "stopped", outcome.receipt()),
            Err(failure) => managed_failure(command, instance_id, &failure),
        },
        PreparedOperation::Destroy {
            instance_id,
            request,
        } => match runtime.destroy_machine(request) {
            Ok(outcome) => machine_success(command, instance_id, "destroyed", outcome.receipt()),
            Err(failure) => managed_failure(command, instance_id, &failure),
        },
    }
}

fn inspection_success(
    command: &'static str,
    instance_id: soma::InstanceId,
    outcome: &soma::MachineInspection,
) -> Execution {
    Execution {
        response: Response::with_receipt(
            command,
            ResultBody::Inspection(InspectionReport {
                instance_id,
                state: outcome.state(),
                backend: outcome.receipt().backend(),
            }),
            outcome.receipt().clone(),
            None,
        ),
        exit: ProcessExit::Success,
    }
}
