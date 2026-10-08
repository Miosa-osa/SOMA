use std::time::Instant;

use soma::{
    BackendFailureKind, DirectCommand, ExecuteMachineRequest, ExecutionLimits, ManagedFailure,
    ManagedStateError, RunFailureKind, TerminalStatus,
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
    Runner, RunnerResponse, Timing, entry, exec_contract, params::ExecParams, refused_sandbox,
    shell, with_journal,
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
        let request = match execute_request(&id, &params, self.config.shell_free_exec) {
            Ok(request) => request,
            Err(error) => {
                self.sandboxes.release(&id);
                return Err(refuse(&error, Some(&owner)));
            }
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
///
/// Only a machine that is genuinely gone is an agent outage. A request the runner or the machine
/// refused is a refusal, and answering one as `AGENT_UNAVAILABLE` told the caller to retry a
/// request that could never succeed while the sandbox it was aimed at had already been released.
pub(super) fn failure_error(failure: &ManagedFailure) -> PlatformError {
    match failure {
        ManagedFailure::State(ManagedStateError::MachineNotFound) => {
            PlatformError::sandbox_not_found()
        }
        ManagedFailure::State(ManagedStateError::MachineStopped) => {
            PlatformError::sandbox_not_running()
        }
        ManagedFailure::Operation(failure) => match failure.kind() {
            RunFailureKind::Backend { kind, .. } => backend_failure_error(kind),
            _ => PlatformError::agent_unavailable(),
        },
        ManagedFailure::Backend(kind) => backend_failure_error(*kind),
        _ => PlatformError::agent_unavailable(),
    }
}

/// The platform error one backend refusal answers with.
///
/// These three are properties of the request rather than of the machine: a workload the sandbox
/// runtime will not take, a second operation against a sandbox already running one, and an
/// operation this backend does not serve. Each is answered by its own code so a caller can tell
/// them apart, and none of them is retryable because asking again changes none of them.
fn backend_failure_error(kind: BackendFailureKind) -> PlatformError {
    match kind {
        BackendFailureKind::WorkloadRejected => PlatformError::new(
            400,
            "WORKLOAD_REJECTED",
            "the sandbox runtime refused this request",
            false,
        ),
        BackendFailureKind::ResourceConflict => PlatformError::new(
            409,
            "RESOURCE_CONFLICT",
            "the sandbox is already running another operation",
            false,
        ),
        BackendFailureKind::Unsupported => PlatformError::new(
            501,
            "BACKEND_UNSUPPORTED",
            "the sandbox runtime does not serve this request",
            false,
        ),
        BackendFailureKind::Unavailable
        | BackendFailureKind::IsolationFailure
        | BackendFailureKind::GuestFailure
        | BackendFailureKind::Timeout
        | BackendFailureKind::OutputLimit
        | BackendFailureKind::CleanupFailure => PlatformError::agent_unavailable(),
    }
}

pub(super) fn command_refusal(unavailable: &Unavailable) -> PlatformError {
    match unavailable {
        Unavailable::NotFound => PlatformError::sandbox_not_found(),
        Unavailable::Busy => PlatformError::exec_busy(),
        Unavailable::Destroyed(..) => PlatformError::sandbox_not_running(),
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
pub(super) fn exec_command(
    text: &str,
    shell_free_exec: bool,
    timeout_millis: u64,
) -> Result<DirectCommand, PlatformError> {
    let (program, arguments) = match shell::plan(text, shell_free_exec) {
        shell::Plan::LoginShell(command) => (shell::SHELL, vec!["-lc", command]),
        shell::Plan::PlainShell(command) => (shell::SHELL, vec!["-c", command]),
        shell::Plan::Direct { program, arguments } => (program, arguments),
    };
    exec_contract::admit(program, &arguments, timeout_millis)
        .map_err(exec_contract::Refusal::error)?;
    DirectCommand::new(program, arguments).map_err(|_| {
        // The contract above is stricter than this type on every field, so a command that got
        // here is one this constructor takes; a refusal from it would be a runner bug rather
        // than a request a caller can fix.
        PlatformError::invalid_param("command", "the command is not one this runner can send")
    })
}

/// The request one exec carries to the facade.
fn execute_request(
    id: &SandboxId,
    params: &ExecParams,
    shell_free_exec: bool,
) -> Result<ExecuteMachineRequest, PlatformError> {
    let command = exec_command(&params.command, shell_free_exec, params.timeout_ms)?;
    let limits = ExecutionLimits::new(params.timeout_ms, ExecutionLimits::DEFAULT_MAX_OUTPUT_BYTES)
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
