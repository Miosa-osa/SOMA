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
