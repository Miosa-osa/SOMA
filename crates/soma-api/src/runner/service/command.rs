use std::time::Instant;

use soma::{
    DirectCommand, ExecuteMachineRequest, ExecutionLimits, ManagedFailure, ManagedStateError,
    TerminalStatus,
};

use crate::{
    facade::CommandOutcome,
    failure::managed_error,
    runner::{
        backend::{Busy, CallTiming},
        ids::SandboxId,
        journal::{Entry, EntryKind},
        keys::Principal,
        public_wire::{self, PlatformError},
        sandboxes::{Owner, Unavailable},
    },
    wire::operation_id,
};

use super::{
    Runner, RunnerResponse, Timing, entry, params::ExecParams, refused_sandbox, with_journal,
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
        let Prepared { id, owner, request } = match self.prepare_command(principal, raw_id, body) {
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
    pub(super) fn prepare_command(
        &self,
        principal: &Principal,
        raw_id: &str,
        body: &[u8],
    ) -> Result<Prepared, Box<RunnerResponse>> {
        let id = self
            .addressed(raw_id)
            .map_err(|response| Box::new(refused_sandbox(*response, EntryKind::Exec, principal)))?;
        let refuse = |error: &PlatformError, owner: Option<&Owner>| {
            Box::new(with_journal(
                RunnerResponse::platform(error),
                Some(command_entry(principal, &id, owner, error.status, None)),
            ))
        };
        let params = ExecParams::parse(body).map_err(|error| refuse(&error, None))?;
        let owner = self
            .sandboxes
            .begin_command(&id, &principal.key.tenant_id)
            .map_err(|unavailable| refuse(&command_refusal(&unavailable), None))?;
        let Some(request) = execute_request(&id, &params) else {
            self.sandboxes.release(&id);
            let error =
                PlatformError::invalid_param("command", "the command exceeds the runner's bounds");
            return Err(refuse(&error, Some(&owner)));
        };
        Ok(Prepared { id, owner, request })
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

/// The answer to one finished command, and its exit code when it exited.
fn executed_answer(
    outcome: Result<(Result<CommandOutcome, ManagedFailure>, CallTiming), Busy>,
    started: Instant,
) -> (RunnerResponse, Option<i32>) {
    let failure = match outcome {
        Ok((Ok(executed), _)) => {
            let TerminalStatus::Exited { code } = executed.status else {
                // The fast lane only ever answered an exited command; anything else decoded as
                // an invalid receipt and was answered as an unavailable agent.
                return (
                    RunnerResponse::platform(&PlatformError::agent_unavailable()),
                    None,
                );
            };
            let stdout = String::from_utf8_lossy(&executed.stdout);
            let stderr = String::from_utf8_lossy(&executed.stderr);
            let body = public_wire::encode(&public_wire::Executed {
                data: public_wire::ExecutedData {
                    exit_code: code,
                    stderr: &stderr,
                    stdout: &stdout,
                },
            });
            let response = RunnerResponse::new(200, body).header(
                "x-miosa-soma-server-tti-us",
                started.elapsed().as_micros().to_string(),
            );
            return (response, Some(code));
        }
        Ok((Err(failure), _)) => failure,
        Err(Busy) => return (RunnerResponse::runtime_busy(), None),
    };
    let error = failure_error(&failure);
    (RunnerResponse::platform(&error), None)
}

/// The platform error one failed command answers with.
pub(super) fn failure_error(failure: &ManagedFailure) -> PlatformError {
    match failure {
        ManagedFailure::State(ManagedStateError::MachineNotFound) => {
            PlatformError::sandbox_not_found()
        }
        ManagedFailure::State(ManagedStateError::MachineStopped) => {
            PlatformError::sandbox_not_running()
        }
        _ => PlatformError::agent_unavailable(),
    }
}

pub(super) fn command_refusal(unavailable: &Unavailable) -> PlatformError {
    match unavailable {
        Unavailable::NotFound => PlatformError::sandbox_not_found(),
        Unavailable::Busy => PlatformError::exec_busy(),
        Unavailable::Destroyed(..) => PlatformError::sandbox_not_running(),
    }
}

/// The command exactly as the fast lane's host executor ran it: `/bin/sh -lc <command>`.
fn execute_request(id: &SandboxId, params: &ExecParams) -> Option<ExecuteMachineRequest> {
    let command = DirectCommand::new("/bin/sh", ["-lc", params.command.as_str()]).ok()?;
    let limits =
        ExecutionLimits::new(params.timeout_ms, ExecutionLimits::DEFAULT_MAX_OUTPUT_BYTES).ok()?;
    Some(ExecuteMachineRequest::new(
        operation_id(None).ok()?,
        id.instance_id()?,
        command,
        limits,
    ))
}

/// The stable code of a facade failure, for logs and the destroy answer's details.
pub(super) fn failure_code(failure: &ManagedFailure) -> &'static str {
    managed_error(failure).body().code
}
