//! The create that waits for its sandbox to be up before it answers.
//!
//! It lives apart from the create that answers early so that each path is one readable piece: this
//! is the one a create-and-run takes, because its command runs on the machine the launch built.

#[allow(
    clippy::wildcard_imports,
    reason = "this module is its parent's body moved wholesale; naming every import would obscure it"
)]
use super::*;

/// The stable codes a launch fails with when the host cannot give the network a create asked for:
/// no broker to lease from (`unsupported_backend`), a broker that refused the policy
/// (`workload_rejected`), or one that cannot be reached (`backend_unavailable`).
const EGRESS_REFUSALS: [&str; 3] = [
    "unsupported_backend",
    "workload_rejected",
    "backend_unavailable",
];

impl Runner {
    /// Launches one sandbox, waits for it, and answers the create that asked for it.
    ///
    /// This is where a create-and-run happens. Its command was built while the create body was
    /// parsed, and it runs here, on the same facade lease and the same call the launch was made
    /// on: the machine is the one that was just launched, and neither the command nor the create
    /// waits for a second lease to become free.
    #[expect(
        clippy::too_many_lines,
        reason = "one create: the launch, the command it may carry, and every way it answers. Its                   pieces are named rather than split further, because the alternative is passing \
                  the launch context between them as arguments"
    )]
    pub(super) async fn create_and_wait(
        &self,
        principal: &Principal,
        prepared: &PreparedCreate,
        timing: &mut Timing,
    ) -> RunnerResponse {
        let id = &prepared.id;
        let journal = |status: u16, sandbox: Option<&SandboxId>, project: Option<String>| {
            Some(entry(
                EntryKind::Create,
                principal,
                project,
                sandbox,
                status,
            ))
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
        // The machine this builds belongs to the record, so the launch checks the record is
        // still there when it finishes: see `abandoned_launch`.
        let sandboxes = std::sync::Arc::clone(&self.sandboxes);
        let owned = prepared.id.clone();
        let outcome = self
            .backend
            .call(move |facade| {
                if !facade.hosts_addressable_sandboxes() {
                    return Err("durable_machine_hosting_missing");
                }
                // The receipt comes back so the answer can report the phases
                // the launch evidence already timed; the runner itself does
                // not read it for anything else.
                let started = Instant::now();
                let receipt = facade
                    .launch(launch)
                    .map(|launched| launched.receipt)
                    .map_err(|failure| failure::launch_failed(&owned, started, &failure))?;
                if !sandboxes.holds(&owned) {
                    return Err(abandoned_launch(facade, &owned));
                }
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
                timing.launch_preparation = LaunchPreparation::of(created.receipt.preparation());
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
                    journal(201, Some(id), prepared.project.clone()).map(|mut entry| {
                        entry.launch = timing.launch_preparation;
                        entry
                    }),
                )
            }
            Ok((Err(code), call)) => {
                timing.call = call;
                self.sandboxes.abandon(id);
                eprintln!("soma-api: runner create {id} failed: {code}");
                // A create that asked for a network this host cannot give is refused for that,
                // once and plainly. The generic answer below is retryable, and a caller that
                // retries a refused network gets the same refusal, or on another host more
                // egress than it asked for.
                if prepared
                    .params
                    .egress
                    .is_some_and(EgressRequest::needs_broker)
                    && EGRESS_REFUSALS.contains(&code)
                {
                    let refusal = PlatformError::new(
                        409,
                        "egress_unavailable",
                        "this host cannot serve the requested network.egress; no sandbox was created",
                        false,
                    );
                    return with_journal(
                        RunnerResponse::platform(&refusal),
                        journal(409, Some(id), prepared.project.clone()),
                    );
                }
                with_journal(
                    RunnerResponse::create_unavailable(),
                    journal(503, Some(id), prepared.project.clone()),
                )
            }
            Err(_busy) => {
                self.sandboxes.abandon(id);
                with_journal(
                    RunnerResponse::runtime_busy(),
                    journal(429, Some(id), prepared.project.clone()).map(|mut entry| {
                        entry.refusal = Some(RefusalKind::RuntimeBusy);
                        entry
                    }),
                )
            }
        }
    }
}
