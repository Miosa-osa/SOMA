use soma::{CapturedOutput, CommandStatus, ExecutionReceipt, TerminalStatus};

use crate::{
    exit::ProcessExit,
    model::{CommandReport, FailureBody, MachineReport, OutputBytes, Response, ResultBody},
};

use super::Execution;

pub(super) fn command_success(
    command: &'static str,
    instance_id: soma::InstanceId,
    receipt: &ExecutionReceipt,
    output: &CapturedOutput,
) -> Execution {
    let Some(status) = command_status(*receipt.terminal_status()) else {
        return super::failure::software_failure(command, "invalid_terminal_status");
    };
    let (error, exit) = command_result_status(status);
    Execution {
        response: Response::with_receipt(
            command,
            ResultBody::Command(CommandReport {
                instance_id,
                execution: status,
                stdout: OutputBytes::new(output.stdout()),
                stderr: OutputBytes::new(output.stderr()),
            }),
            receipt.clone(),
            error,
        ),
        exit,
    }
}

pub(super) fn machine_success(
    command: &'static str,
    instance_id: soma::InstanceId,
    state: &'static str,
    receipt: &ExecutionReceipt,
) -> Execution {
    Execution {
        response: Response::with_receipt(
            command,
            ResultBody::Machine(MachineReport { instance_id, state }),
            receipt.clone(),
            None,
        ),
        exit: ProcessExit::Success,
    }
}

pub(super) fn success(command: &'static str, result: ResultBody) -> Execution {
    Execution {
        response: Response::success(command, result),
        exit: ProcessExit::Success,
    }
}

pub(super) const fn command_status(terminal: TerminalStatus) -> Option<CommandStatus> {
    match terminal {
        TerminalStatus::Exited { code } => Some(CommandStatus::Exited { code }),
        TerminalStatus::Signaled { signal } => Some(CommandStatus::Signaled { signal }),
        TerminalStatus::TimedOut => Some(CommandStatus::TimedOut),
        TerminalStatus::OutputLimitExceeded => Some(CommandStatus::OutputLimitExceeded),
        _ => None,
    }
}

fn command_result_status(status: CommandStatus) -> (Option<FailureBody>, ProcessExit) {
    match status {
        CommandStatus::Exited { code: 0 } => (None, ProcessExit::Success),
        CommandStatus::Exited { .. } | CommandStatus::Signaled { .. } => (
            Some(FailureBody::new(
                "guest_nonzero",
                "guest command did not exit successfully",
                false,
            )),
            ProcessExit::GuestNonzero,
        ),
        CommandStatus::TimedOut => (
            Some(FailureBody::new(
                "guest_timeout",
                "guest command exceeded its deadline",
                true,
            )),
            ProcessExit::GuestTimeout,
        ),
        CommandStatus::OutputLimitExceeded => (
            Some(FailureBody::new(
                "output_limit",
                "guest output exceeded its declared allowance",
                false,
            )),
            ProcessExit::OutputLimit,
        ),
    }
}
