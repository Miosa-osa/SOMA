use std::time::Instant;

use soma::{CommandContext, DirectCommand, ExecuteMachineRequest, ExecutionLimits, ManagedFailure};

use crate::{
    facade::CommandOutcome,
    failure::managed_error,
    runner::{
        backend::{Busy, CallTiming},
        ids::SandboxId,
        journal::{Entry, EntryKind},
        keys::Principal,
        public_wire::{self, PlatformError},
        sandboxes::Owner,
    },
    wire::operation_id,
};

use super::{
    Runner, RunnerResponse, Timing, entry, exec_contract, outcome, params::ExecParams,
    refused_sandbox, shell, with_journal,
};

/// A command that passed every check and now holds its sandbox's lifecycle slot.
pub(super) struct Prepared {
    pub(super) id: SandboxId,
    pub(super) owner: Owner,
    pub(super) request: ExecuteMachineRequest,
}

impl Runner {
    pub(super) async fn exec(
        &self,
        principal: &Principal,
        raw_id: &str,
        body: &[u8],
        timing: &mut Timing,
    ) -> RunnerResponse {
        let id = match self.addressed(raw_id) {
            Ok(id) => id,
            Err(response) => {
                return refused_sandbox(*response, EntryKind::Exec, principal);
            }
        };
        // A sandbox whose create answered before it was up is waited for here, before the command
        // is built: the command may not be admitted against a machine that does not exist yet.
        let waited = match self.wait_for_launch(&id, &principal.key.tenant_id).await {
            Ok(waited) => waited,
            Err(response) => {
                let status = response.status;
                return with_journal(
                    *response,
                    Some(command_entry(principal, &id, None, status, None)),
                );
            }
        };
        timing.wait = waited;
        let Prepared { id, owner, request } = match self.prepare_command(principal, &id, body) {
            Ok(prepared) => prepared,
            Err(response) => return *response,
        };
        let started = Instant::now();
        let outcome = self
            .backend
            .call(move |facade| facade.execute(request))
            .await;
        self.sandboxes.release(&id);
        if let Ok((_, call)) = &outcome {
            timing.call = *call;
        }
        let (response, exit_code) = executed_answer(outcome, started);
        let status = response.status;
        with_journal(
            response,
            Some(command_entry(
                principal,
                &id,
                Some(&owner),
                status,
                exit_code,
            )),
        )
    }

    /// The checks every command passes, shared by exec and exec/stream. A refusal comes back
    /// as the finished, journaled answer.
    ///
    /// The sandbox is addressed by the caller rather than from a raw id, because a caller has to
    /// wait for a launch its sandbox is still in before the command may be built.
    pub(super) fn prepare_command(
        &self,
        principal: &Principal,
        id: &SandboxId,
        body: &[u8],
    ) -> Result<Prepared, Box<RunnerResponse>> {
        let refuse = |response: RunnerResponse, owner: Option<&Owner>| {
            let status = response.status;
            Box::new(with_journal(
                response,
                Some(command_entry(principal, id, owner, status, None)),
            ))
        };
        let params =
            ExecParams::parse(body).map_err(|error| refuse(RunnerResponse::platform(&error), None))?;
        let owner = self
            .sandboxes
            .begin_command(id, &principal.key.tenant_id)
            .map_err(|unavailable| refuse(outcome::command_refusal(&unavailable), None))?;
        let request = match execute_request(id, &params, self.config.shell_free_exec) {
            Ok(request) => request,
            Err(error) => {
                self.sandboxes.release(id);
                return Err(refuse(RunnerResponse::platform(&error), Some(&owner)));
            }
        };
        Ok(Prepared {
            id: id.clone(),
            owner,
            request,
        })
    }
}

/// The journal entry of one command (contract C4 kind `exec`).
pub(super) fn command_entry(
    principal: &Principal,
    id: &SandboxId,
    owner: Option<&Owner>,
    status: u16,
    exit_code: Option<i32>,
) -> Entry {
    let mut entry = entry(
        EntryKind::Exec,
        principal,
        owner.and_then(|owner| owner.project_id.clone()),
        Some(id),
        status,
    );
    entry.exit_code = exit_code;
    entry
}

/// What one command produced, in the terms the exec endpoint answers it in.
pub(super) enum Ran {
    /// The command ended: its exit code, and the output it produced.
    Exited {
        exit_code: i32,
        stdout: String,
        stderr: String,
    },
    /// The command produced no result, and this is the answer the exec endpoint gives.
    Failed(PlatformError),
}

/// Reads one finished facade command call.
///
/// This is the whole of the rule that turns a command's outcome into an answer: a command that
/// ended has an exit code, a command that produced no result is answered by what stopped it, and
/// a command the engine named is answered by [`outcome::failure_error`]. Every door onto a
/// command reads it here, so `exec` and a create-and-run can never disagree about one.
pub(super) fn ran(outcome: Result<CommandOutcome, ManagedFailure>) -> Ran {
    match outcome {
        Ok(executed) => match outcome::exit_code(executed.status) {
            Some(exit_code) => Ran::Exited {
                exit_code,
                stdout: String::from_utf8_lossy(&executed.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&executed.stderr).into_owned(),
            },
            // The fast lane only ever answered a command that ended; anything else decoded as an
            // invalid receipt and was answered as an unavailable agent.
            None => Ran::Failed(PlatformError::agent_unavailable()),
        },
        Err(failure) => Ran::Failed(outcome::failure_error(&failure)),
    }
}

/// The answer to one finished command, and its exit code when it exited.
fn executed_answer(
    outcome: Result<(Result<CommandOutcome, ManagedFailure>, CallTiming), Busy>,
    started: Instant,
) -> (RunnerResponse, Option<i32>) {
    let outcome = match outcome {
        Ok((command, _)) => command,
        Err(Busy) => return (RunnerResponse::runtime_busy(), None),
    };
    match ran(outcome) {
        Ran::Exited {
            exit_code,
            stdout,
            stderr,
        } => {
            let body = public_wire::encode(&public_wire::Executed {
                data: public_wire::ExecutedData {
                    exit_code,
                    stderr: &stderr,
                    stdout: &stdout,
                },
            });
            let response = RunnerResponse::new(200, body).header(
                "x-miosa-soma-server-tti-us",
                started.elapsed().as_micros().to_string(),
            );
            (response, Some(exit_code))
        }
        Ran::Failed(error) => (RunnerResponse::platform(&error), None),
    }
}

/// One command's argv, chosen by [`shell::plan`] and admitted by the machine's exec contract.
///
/// With `shell_free_exec` off this is the fast lane's own `/bin/sh -lc <command>` for every
/// command. With it on, a command that is only whitespace-separated words runs with no shell at
/// all, and everything else runs under `/bin/sh -c` without sourcing a profile.
///
/// The plan says how the command would reach the guest; [`exec_contract::admit`] says whether
/// anything will carry it. Both answers have to be given here, because a command the guest would
/// refuse is only refusable while no machine has been asked to run it.
#[cfg(test)]
pub(super) fn exec_command(
    text: &str,
    shell_free_exec: bool,
    timeout_millis: u64,
) -> Result<DirectCommand, PlatformError> {
    exec_command_in(
        text,
        shell_free_exec,
        timeout_millis,
        &CommandContext::default(),
    )
}

/// [`exec_command`] for a command that carries an environment, working directory or user.
pub(super) fn exec_command_in(
    text: &str,
    shell_free_exec: bool,
    timeout_millis: u64,
    context: &CommandContext,
) -> Result<DirectCommand, PlatformError> {
    let (program, arguments) = match shell::plan(text, shell_free_exec) {
        shell::Plan::LoginShell(command) => (shell::SHELL, vec!["-lc", command]),
        shell::Plan::PlainShell(command) => (shell::SHELL, vec!["-c", command]),
        shell::Plan::Direct { program, arguments } => (program, arguments),
    };
    exec_contract::admit_in(program, &arguments, timeout_millis, context)
        .map_err(exec_contract::Refusal::error)?;
    let command = DirectCommand::new(program, arguments).map_err(|_| {
        // The contract above is stricter than this type on every field, so a command that got
        // here is one this constructor takes; a refusal from it would be a runner bug rather
        // than a request a caller can fix.
        PlatformError::invalid_param("command", "the command is not one this runner can send")
    })?;
    // `admit_in` already ran the guest's own constructor over the whole command, context
    // included, so this only moves the verified context onto the facade's command.
    command.with_context(context.clone()).map_err(|_| {
        PlatformError::invalid_param("command", "the command is not one this runner can send")
    })
}

/// The request one exec carries to the facade.
fn execute_request(
    id: &SandboxId,
    params: &ExecParams,
    shell_free_exec: bool,
) -> Result<ExecuteMachineRequest, PlatformError> {
    let command = exec_command_in(
        &params.command,
        shell_free_exec,
        params.timeout_ms,
        &params.context,
    )?;
    command_request(id, command, params.timeout_ms)
}

/// The request one already-built command carries to the facade.
///
/// A create-and-run builds its command while it parses the create body, so this is the second
/// half of the same construction rather than a second way to build one.
pub(super) fn command_request(
    id: &SandboxId,
    command: DirectCommand,
    timeout_ms: u64,
) -> Result<ExecuteMachineRequest, PlatformError> {
    let limits = ExecutionLimits::new(timeout_ms, ExecutionLimits::DEFAULT_MAX_OUTPUT_BYTES)
        .map_err(|_| PlatformError::invalid_timeout())?;
    let unbuildable = || PlatformError::invalid_param("command", "the command could not be built");
    let operation = operation_id(None).map_err(|_| unbuildable())?;
    let instance = id.instance_id().ok_or_else(unbuildable)?;
    Ok(ExecuteMachineRequest::new(
        operation, instance, command, limits,
    ))
}

/// The stable code of a facade failure, for logs and the destroy answer's details.
pub(super) fn failure_code(failure: &ManagedFailure) -> &'static str {
    managed_error(failure).body().code
}
