use std::path::PathBuf;

use soma_local::{LocalRuntime, LocalRuntimeConfig, MachineHosting};

use crate::{
    cli::BackendSelection,
    exit::ProcessExit,
    model::{FileReport, InspectionReport, PtyReport, Response, ResultBody, SandboxListReport},
    request::PreparedOperation,
};

use super::{
    Execution,
    failure::{listing_failure, local_failure, managed_failure, not_hosted, run_failure},
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
        // A launch this process cannot outlive is refused before a machine is built rather than
        // reported ready. The command line hands its instance identity to a caller who will use
        // it from a second process, and on a backend that hosts the machine here there is
        // nothing for that second process to reach. Refusing costs the caller a machine it could
        // never have used; reporting ready costs it a retry loop against a dead identity.
        PreparedOperation::Launch { .. }
            if runtime.machine_hosting() == MachineHosting::LaunchingProcess =>
        {
            not_hosted(command)
        }
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
        PreparedOperation::File {
            instance_id,
            request,
        } => match runtime.file_machine(request) {
            Ok(outcome) => file_success(command, instance_id, &outcome),
            Err(failure) => managed_failure(command, instance_id, &failure),
        },
        PreparedOperation::Pty {
            instance_id,
            request,
        } => match runtime.pty_machine(request) {
            Ok(outcome) => pty_success(command, instance_id, &outcome),
            Err(failure) => managed_failure(command, instance_id, &failure),
        },
        PreparedOperation::List => match runtime.list_machines() {
            Ok(entries) => Execution {
                response: Response::success(
                    command,
                    ResultBody::List(SandboxListReport::new(&entries)),
                ),
                exit: ProcessExit::Success,
            },
            Err(failure) => listing_failure(command, &failure),
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

/// Reports one filesystem answer.
///
/// A refusal the guest reported is a failed command line operation rather than a successful one:
/// a script that wrote to a read-only root must not see a zero exit. The report is still carried,
/// because the typed cause is the useful part of the answer.
fn file_success(
    command: &'static str,
    instance_id: soma::InstanceId,
    outcome: &soma::MachineFile,
) -> Execution {
    let report = FileReport::new(instance_id, outcome.operation().name(), outcome.answer());
    let refused = report.refused();
    let body = ResultBody::File(report);
    if refused {
        return Execution {
            response: Response::failure_with_result(
                command,
                body,
                crate::model::FailureBody::new(
                    "filesystem_refused",
                    "the guest declined the filesystem operation",
                    false,
                ),
            ),
            exit: ProcessExit::Conflict,
        };
    }
    Execution {
        response: Response::success(command, body),
        exit: ProcessExit::Success,
    }
}

/// Reports one terminal answer.
///
/// A refusal the guest reported is a failed command line operation rather than a successful one,
/// for the reason a filesystem refusal is: a script that wrote to a terminal that is not open
/// must not see a zero exit.
fn pty_success(
    command: &'static str,
    instance_id: soma::InstanceId,
    outcome: &soma::MachinePty,
) -> Execution {
    let report = PtyReport::new(instance_id, outcome.operation().name(), outcome.answer());
    let refused = report.refused();
    let body = ResultBody::Pty(report);
    if refused {
        return Execution {
            response: Response::failure_with_result(
                command,
                body,
                crate::model::FailureBody::new(
                    "terminal_refused",
                    "the guest declined the terminal operation",
                    false,
                ),
            ),
            exit: ProcessExit::Conflict,
        };
    }
    Execution {
        response: Response::success(command, body),
        exit: ProcessExit::Success,
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
