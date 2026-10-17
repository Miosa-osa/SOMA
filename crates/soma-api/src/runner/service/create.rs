use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use soma::{ExecutionReceipt, LaunchMachineRequest, MachineName, MachineShape};

use crate::{
    facade::CommandOutcome,
    runner::{
        backend::Busy,
        clock::iso8601_micros,
        idle::Lifetime,
        ids::SandboxId,
        journal::{Entry, EntryKind},
        keys::Principal,
        public_wire::{self, PlatformError},
        sandboxes::{LaunchFailure, Owner},
        starting::{LaunchState, Starting},
    },
    wire::operation_id,
};

use super::{
    Runner, RunnerResponse, TENANT_LABEL_PREFIX, Timing, command, entry, failure_code, millis,
    params::CreateParams, timing::LaunchPhases, with_journal,
};

mod early;

/// One create, prepared to the point where all that is left is its launch.
///
/// The two doors a create can take from here differ only in when they answer; everything before
/// the launch, and everything the answer is made of, is the same for both, so it is gathered once
/// rather than passed as six arguments each.
struct PreparedCreate {
    /// The sandbox the runner minted for this create.
    id: SandboxId,
    /// The launch that brings its machine up.
    launch: LaunchMachineRequest,
    /// What the request asked for, which the answer is made from.
    params: CreateParams,
    /// The project the create was for, for the journal.
    project: Option<String>,
}

impl Runner {
    pub(super) async fn create(
        &self,
        principal: &Principal,
        body: &[u8],
        timing: &mut Timing,
    ) -> RunnerResponse {
        let journal = |status: u16, sandbox: Option<&SandboxId>, project: Option<String>| {
            Some(entry(
                EntryKind::Create,
                principal,
                project,
                sandbox,
                status,
            ))
        };
        // Every segment is additive to the header, so the runner's own work is
        // timed here rather than inferred from the client's view of the request.
        let prep_started = Instant::now();
        let params = match self.create_params(principal, body) {
            Ok(params) => params,
            Err(response) => {
                let status = response.status;
                return with_journal(*response, journal(status, None, None));
            }
        };
        let project = params.project_id.clone();
        let Ok(_permit) = self.admission.try_acquire() else {
            return with_journal(RunnerResponse::runtime_busy(), journal(429, None, project));
        };
        // The tenant's share on this runner is one atomic counter (contract C3).
        if !principal.tenant.admit() {
            return with_journal(
                RunnerResponse::refusal(429, "rate_limited"),
                journal(429, None, project),
            );
        }
        let id = SandboxId::mint(self.config.host_tag);
        let owner = Owner {
            tenant_id: principal.key.tenant_id.clone(),
            key_id: Some(principal.key.key_id.clone()),
            project_id: project.clone(),
            created: Instant::now(),
            slot: principal.tenant.counter(),
        };
        let lifetime = Lifetime::from_seconds(
            params.timeout_seconds,
            principal.tenant.policy.max_lifetime_seconds,
        );
        self.sandboxes.reserve(id.clone(), owner, lifetime);
        let Some(launch) = self.launch_request(&id, &principal.key.tenant_id, &params.shape) else {
            self.sandboxes.abandon(&id);
            return with_journal(
                RunnerResponse::create_unavailable(),
                journal(503, Some(&id), project),
            );
        };
        timing.prep = prep_started.elapsed();
        let prepared = PreparedCreate {
            id,
            launch,
            params,
            project,
        };
        if prepared.params.wait_ready {
            self.create_and_wait(principal, &prepared, timing).await
        } else {
            self.create_before_ready(principal, &prepared, timing).await
        }
    }

    /// Launches one sandbox, waits for it, and answers the create that asked for it.
    ///
    /// This is where a create-and-run happens. Its command was built while the create body was
    /// parsed, and it runs here, on the same facade lease and the same call the launch was made
    /// on: the machine is the one that was just launched, and neither the command nor the create
    /// waits for a second lease to become free.
    async fn create_and_wait(
        &self,
        principal: &Principal,
        prepared: &PreparedCreate,
        timing: &mut Timing,
    ) -> RunnerResponse {
        let id = &prepared.id;
        let journal = |status: u16, sandbox: Option<&SandboxId>, project: Option<String>| {
            Some(entry(EntryKind::Create, principal, project, sandbox, status))
        };
        let request = match prepared
            .params
            .run
            .as_ref()
            .map(|run| command::command_request(id, run.command.clone(), run.timeout_ms))
        {
            None => None,
            Some(Ok(request)) => Some(request),
            Some(Err(error)) => {
                // Unreachable: the command was built and admitted against the machine's contract
                // while the body was parsed. A runner bug is answered as one rather than as a
                // sandbox this request then has to account for.
                self.sandboxes.abandon(id);
                return with_journal(
                    RunnerResponse::platform(&error),
                    journal(error.status, Some(id), prepared.project.clone()),
                );
            }
        };
        let launch = prepared.launch.clone();
        let outcome = self
            .backend
            .call(move |facade| {
                if !facade.hosts_addressable_sandboxes() {
                    return Err("durable_machine_hosting_missing");
                }
                // The receipt comes back so the answer can report the phases
                // the launch evidence already timed; the runner itself does
                // not read it for anything else.
                let receipt = facade
                    .launch(launch)
                    .map(|launched| launched.receipt)
                    .map_err(|failure| failure_code(&failure))?;
                let run = request.map(|request| {
                    let started = Instant::now();
                    let outcome = facade.execute(request);
                    (started.elapsed(), outcome)
                });
                Ok(CreateOutcome { receipt, run })
            })
            .await;
        match outcome {
            Ok((Ok(created), call)) => {
                timing.call = call;
                let ran = created.run.map(|(duration, outcome)| {
                    timing.run = Some(duration);
                    (millis(duration), command::ran(outcome))
                });
                // The command is part of this call, so the launch evidence's remainder is what is
                // left of `exec` once the command's own segment is taken out of it.
                let launch_only = match &ran {
                    Some((ms, _)) => call.exec.saturating_sub(Duration::from_millis(*ms)),
                    None => call.exec,
                };
                timing.launch = LaunchPhases::from_receipt(&created.receipt, launch_only);
                let finish_started = Instant::now();
                self.sandboxes.confirm(id);
                let body = self.created_body(
                    id,
                    &prepared.params,
                    millis(call.pool + call.exec),
                    "running",
                    ran.as_ref(),
                );
                timing.finish = finish_started.elapsed();
                let runner_url = self.runner_url(self.config.host_tag);
                with_journal(
                    RunnerResponse::new(201, body).header("soma-runner-url", runner_url),
                    journal(201, Some(id), prepared.project.clone()),
                )
            }
            Ok((Err(code), call)) => {
                timing.call = call;
                self.sandboxes.abandon(id);
                eprintln!("soma-api: runner create {id} failed: {code}");
                with_journal(
                    RunnerResponse::create_unavailable(),
                    journal(503, Some(id), prepared.project.clone()),
                )
            }
            Err(_busy) => {
                self.sandboxes.abandon(id);
                with_journal(
                    RunnerResponse::runtime_busy(),
                    journal(429, Some(id), prepared.project.clone()),
                )
            }
        }
    }

    /// The checks a create passes before it may take an admission slot.
    fn create_params(
        &self,
        principal: &Principal,
        body: &[u8],
    ) -> Result<CreateParams, Box<RunnerResponse>> {
        if !self.keys.has_received()
            || self.keys.resync_required()
            || self.keys.feed_age(Instant::now()) > self.config.feed_stale_after()
        {
            return Err(Box::new(RunnerResponse::feed_stale()));
        }
        // C7: the request's timeout, else the tenant's default, else this runner's.
        let default_timeout = principal
            .tenant
            .policy
            .default_timeout_seconds
            .unwrap_or(self.config.launch.default_timeout_seconds);
        let params = CreateParams::parse(
            body,
            default_timeout,
            &self.config.launch,
            self.config.shell_free_exec,
        )
        .map_err(|error| Box::new(RunnerResponse::platform(&error)))?;
        if !principal.key.projects.allows(params.project_id.as_deref()) {
            return Err(Box::new(RunnerResponse::refusal(403, "forbidden")));
        }
        Ok(params)
    }

    /// The `201` body one create answers with.
    ///
    /// `state` is what the sandbox's own state is when the answer is formed: `running` for a
    /// create that waited, and `starting` for one that answered while its launch was still in
    /// flight. `ran` is present only for a create-and-run, and only a create-and-run's body
    /// carries a `run` key at all.
    fn created_body(
        &self,
        id: &SandboxId,
        params: &CreateParams,
        create_ms: u64,
        state: &str,
        ran: Option<&(u64, command::Ran)>,
    ) -> Vec<u8> {
        let created_at = iso8601_micros(SystemTime::now());
        let ran = ran.map(|(duration_ms, ran)| ran_object(*duration_ms, ran));
        public_wire::encode(&public_wire::Created {
            cpu_count: params.shape.vcpu_count(),
            create_ms,
            created_at: &created_at,
            deletion_pending: false,
            id: id.as_str(),
            memory_mb: params.shape.memory_mib(),
            name: None,
            run: ran.as_ref(),
            runner_url: &self.runner_url(self.config.host_tag),
            slug: &id.as_str()[..8],
            state,
            template_id: &self.config.launch.template_id,
            timeout_sec: params.timeout_seconds,
        })
    }

    fn launch_request(
        &self,
        id: &SandboxId,
        tenant_id: &str,
        shape: &MachineShape,
    ) -> Option<LaunchMachineRequest> {
        let instance_id = id.instance_id()?;
        let operation = operation_id(None).ok()?;
        let request =
            LaunchMachineRequest::new(operation, instance_id, self.image.clone(), shape.clone());
        Some(
            match MachineName::parse(format!("{TENANT_LABEL_PREFIX}{tenant_id}")) {
                Ok(name) => request.with_name(name),
                Err(_) => request,
            },
        )
    }
}

/// The journal entry of one create, whatever it answered.
fn created_entry(principal: &Principal, prepared: &PreparedCreate, status: u16) -> Entry {
    entry(
        EntryKind::Create,
        principal,
        prepared.project.clone(),
        Some(&prepared.id),
        status,
    )
}

/// What one create's facade call produced: its launch, and the command a create-and-run ran.
struct CreateOutcome {
    receipt: ExecutionReceipt,
    /// The command's own duration, and what it produced; absent when the create carried none.
    run: Option<(Duration, Result<CommandOutcome, soma::ManagedFailure>)>,
}

/// The `run` object a create-and-run publishes.
///
/// A command that ended is reported as the exec endpoint reports it: the exit code and the output.
/// A command that produced no result carries the `error` object the exec endpoint would have
/// answered with, read back out of that answer rather than rebuilt here, so the two can never
/// drift apart, and `exit_code` is null to say the command produced no result of its own.
fn ran_object(duration_ms: u64, ran: &command::Ran) -> public_wire::Ran<'_> {
    match ran {
        command::Ran::Exited {
            exit_code,
            stdout,
            stderr,
        } => public_wire::Ran {
            duration_ms,
            error: None,
            exit_code: Some(*exit_code),
            stderr,
            stdout,
        },
        command::Ran::Failed(error) => public_wire::Ran {
            duration_ms,
            error: Some(error_object(error)),
            exit_code: None,
            stderr: "",
            stdout: "",
        },
    }
}

/// The `error` object of one platform answer, as that answer itself writes it.
fn error_object(error: &PlatformError) -> serde_json::Value {
    serde_json::from_slice::<serde_json::Value>(&error.body())
        .ok()
        .and_then(|body| body.get("error").cloned())
        .unwrap_or(serde_json::Value::Null)
}
