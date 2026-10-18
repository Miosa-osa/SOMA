//! The create that answers before its sandbox is up.
//!
//! It lives apart from the create that waits so that each path is one readable piece: this is the
//! one that hands a launch to the sandbox's own record and answers the request that started it.

#[allow(
    clippy::wildcard_imports,
    reason = "this module is its parent's body moved wholesale; naming every import would obscure it"
)]
use super::*;

impl Runner {
    /// Answers a create as soon as its launch is under way, and leaves the launch to the sandbox.
    ///
    /// The sandbox's own record holds the launch from here, so every later call on it waits for
    /// the launch to end and reads what it reported, and the sandbox's lifecycle (its idle
    /// lifetime, its tenant slot, its destroy) is that of a sandbox that exists rather than of a
    /// request that is still running. Nothing about the launch itself changes: it is the same
    /// call, on the same lease, that a synchronous create would have waited for.
    pub(super) async fn create_before_ready(
        &self,
        principal: &Principal,
        prepared: &PreparedCreate,
        timing: &mut Timing,
        reservation: &mut Reservation,
    ) -> RunnerResponse {
        let (sender, mut create) = Starting::channel();
        self.sandboxes.start_launch(&prepared.id, create.clone());
        // The record belongs to the launch from here, so it is no longer this request's to give
        // back: its slot is the sandbox's now, and the launch settles it either way.
        reservation.disarm();
        self.spawn_launch(prepared, sender);
        // The answer's own timing reports the wait for the launch to take its lease as its `pool`
        // segment, and its `exec` is zero because this request's facade call had not returned; the
        // launch's phases are zero for the same reason, and appear in the `wait` segment of the
        // first call that waits for them.
        let waited = Instant::now();
        let begun = create.begun().await;
        timing.call.pool = waited.elapsed();
        timing.call.exec = Duration::ZERO;
        let state = match begun {
            // The launch ended before this answer was formed, so the answer says what is true: the
            // sandbox is up, and its launch cost what the receipt it returned says it cost.
            LaunchState::Ready { receipt, call } => {
                timing.call = call;
                timing.launch = LaunchPhases::from_receipt(&receipt, call.exec);
                "running"
            }
            // The create is answered here, so the failure is this request's to report: the
            // statuses are the ones a synchronous create answers for the same failure, which is
            // what the sandbox's own record then reads back to every later call on it.
            LaunchState::Busy => {
                return with_journal(
                    RunnerResponse::runtime_busy(),
                    Some(created_entry(principal, prepared, 429)),
                );
            }
            LaunchState::Failed(code) => {
                eprintln!("soma-api: runner create {} failed: {code}", prepared.id);
                return with_journal(
                    RunnerResponse::create_unavailable(),
                    Some(created_entry(principal, prepared, 503)),
                );
            }
            LaunchState::Lost => {
                eprintln!(
                    "soma-api: runner create {} was lost before it reported",
                    prepared.id
                );
                return with_journal(
                    RunnerResponse::create_unavailable(),
                    Some(created_entry(principal, prepared, 503)),
                );
            }
            // Under way, and `begun` returns nothing else: an arm for the type system, not for a
            // caller.
            LaunchState::Launched | LaunchState::Pending => "starting",
        };
        let finish_started = Instant::now();
        let body = self.created_body(
            &prepared.id,
            &prepared.params,
            millis(timing.call.pool + timing.call.exec),
            state,
            None,
        );
        timing.finish = finish_started.elapsed();
        let runner_url = self.runner_url(self.config.host_tag);
        with_journal(
            RunnerResponse::new(201, body).header("soma-runner-url", runner_url),
            Some(created_entry(principal, prepared, 201)),
        )
    }

    /// Starts the launch a create answered early for, and files what it reports against the
    /// sandbox, so every later call can wait for it and read it.
    pub(super) fn spawn_launch(
        &self,
        prepared: &PreparedCreate,
        sender: tokio::sync::watch::Sender<LaunchState>,
    ) {
        let backend = self.backend.clone();
        let sandboxes = Arc::clone(&self.sandboxes);
        let id = prepared.id.clone();
        let launch = prepared.launch.clone();
        let launch_id = prepared.id.clone();
        tokio::spawn(async move {
            let lease = sender.clone();
            let outcome = backend
                .spawn(move |facade| {
                    let _reported = lease.send(LaunchState::Launched);
                    if facade.hosts_addressable_sandboxes() {
                        let started = Instant::now();
                        facade
                            .launch(launch)
                            .map(|launched| launched.receipt)
                            .map_err(|failure| {
                                super::failure::launch_failed(&launch_id, started, &failure)
                            })
                    } else {
                        Err("durable_machine_hosting_missing")
                    }
                })
                .await;
            let end = match outcome {
                Ok(Ok((Ok(receipt), call))) => {
                    sandboxes.confirm(&id);
                    LaunchState::Ready {
                        receipt: Box::new(receipt),
                        call,
                    }
                }
                Ok(Ok((Err(code), _))) => {
                    // The slot goes back now, as it does for a failed synchronous create; the
                    // record stays so the failure is readable rather than a `404` that hides it.
                    sandboxes.fail_launch(&id, LaunchFailure::Unavailable { code });
                    eprintln!("soma-api: runner create {id} failed: {code}");
                    LaunchState::Failed(code)
                }
                Ok(Err(Busy)) => {
                    sandboxes.fail_launch(&id, LaunchFailure::Busy);
                    LaunchState::Busy
                }
                Err(_join) => {
                    sandboxes.fail_launch(
                        &id,
                        LaunchFailure::Unavailable {
                            code: "launch_task_lost",
                        },
                    );
                    LaunchState::Lost
                }
            };
            let _sent = sender.send(end);
        });
    }
}
