//! What one create answers with: its body, its paperwork, and what its command contributed.
//!
//! It is separated from the requests that produce it because the shape of the answer is a
//! contract with the platform while the order of checks is this runner's own business.

#[allow(
    clippy::wildcard_imports,
    reason = "this module is its parent's body moved wholesale; naming every import would obscure it"
)]
use super::*;

impl Runner {
    /// The `201` body one create answers with.
    ///
    /// `state` is what the sandbox's own state is when the answer is formed: `running` for a
    /// create that waited, and `starting` for one that answered while its launch was still in
    /// flight. `ran` is present only for a create-and-run, and only a create-and-run's body
    /// carries a `run` key at all.
    pub(super) fn created_body(
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
            network: params.egress.map(|egress| public_wire::NetworkEcho {
                egress: egress.word(),
                windowed: params.windowed.then_some(true),
            }),
            run: ran.as_ref(),
            runner_url: &self.runner_url(self.config.host_tag),
            slug: &id.as_str()[..8],
            state,
            template_id: &self.config.launch.template_id,
            timeout_sec: params.timeout_seconds,
        })
    }
}

/// The journal entry of one create, whatever it answered.
pub(super) fn created_entry(
    principal: &Principal,
    prepared: &PreparedCreate,
    status: u16,
) -> Entry {
    entry(
        EntryKind::Create,
        principal,
        prepared.project.clone(),
        Some(&prepared.id),
        status,
    )
}

/// What one create's facade call produced: its launch, and the command a create-and-run ran.
pub(super) struct CreateOutcome {
    pub(super) receipt: ExecutionReceipt,
    /// The command's own duration, and what it produced; absent when the create carried none.
    pub(super) run: Option<(Duration, Result<CommandOutcome, soma::ManagedFailure>)>,
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

impl Runner {
    /// The checks a create passes before it may take an admission slot.
    pub(super) fn create_params(
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
}

/// Takes down a machine whose create request went away before it finished being built.
///
/// The machine belongs to the sandbox record, and the record went with the request. A machine no
/// record reaches is never listed, never destroyed and never accounted for, so the launch that
/// built it is the last thing that can take it down. Returns the code that launch reports.
pub(super) fn abandoned_launch(
    facade: &mut dyn crate::facade::SandboxFacade,
    id: &SandboxId,
) -> &'static str {
    // The log says what happened to the machine, not what was hoped for: a destroy that failed
    // leaves a machine nothing owns, and that is the line an operator needs to see.
    let outcome = match (id.instance_id(), operation_id(None)) {
        (Some(instance), Ok(operation)) => {
            match facade.destroy(DestroyMachineRequest::new(operation, instance)) {
                Ok(_) => "the machine was destroyed".to_owned(),
                Err(failure) => {
                    format!("destroying the machine failed: {}", failure_code(&failure))
                }
            }
        }
        (None, _) => "the sandbox has no instance id, so no machine was destroyed".to_owned(),
        (_, Err(_)) => {
            "no operation id could be minted, so the machine was not destroyed".to_owned()
        }
    };
    eprintln!("soma-api: runner create {id} was abandoned while its machine was built; {outcome}");
    "create_abandoned"
}
