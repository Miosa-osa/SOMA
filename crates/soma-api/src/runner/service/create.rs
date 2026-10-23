use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use crate::runner::journal::RefusalKind;
use soma::{
    DestroyMachineRequest, ExecutionReceipt, LaunchMachineRequest, MachineName, MachineShape,
};

use crate::{
    facade::CommandOutcome,
    runner::{
        backend::Busy,
        clock::iso8601_micros,
        idle::Lifetime,
        ids::SandboxId,
        journal::{Entry, EntryKind, LaunchPreparation},
        keys::Principal,
        public_wire::{self, PlatformError},
        sandboxes::{LaunchFailure, Owner},
        starting::{LaunchState, Starting},
    },
    wire::operation_id,
};

use crate::runner::sandboxes::Reservation;

use super::{
    Runner, RunnerResponse, TENANT_LABEL_PREFIX, Timing, command, entry, failure_code, millis,
    params::{CreateParams, EgressRequest},
    timing::LaunchPhases,
    with_journal,
};

/// The stable codes a launch fails with when the host cannot give the network a create asked for:
/// no broker to lease from (`unsupported_backend`), a broker that refused the policy
/// (`workload_rejected`), or one that cannot be reached (`backend_unavailable`).
const EGRESS_REFUSALS: [&str; 3] = [
    "unsupported_backend",
    "workload_rejected",
    "backend_unavailable",
];

mod body;
mod early;
mod failure;

// Its own and its sibling module's, so `early` reads them from `super::*` as it did when they
// were written here.
use body::{CreateOutcome, abandoned_launch, created_entry};

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
            return with_journal(
                RunnerResponse::runtime_busy(),
                journal(429, None, project).map(|mut entry| {
                    entry.refusal = Some(RefusalKind::RuntimeBusy);
                    entry
                }),
            );
        };
        // The tenant's share on this runner is one atomic counter (contract C3).
        if !principal.tenant.admit() {
            return with_journal(
                RunnerResponse::refusal(429, "rate_limited"),
                journal(429, None, project).map(|mut entry| {
                    entry.refusal = Some(RefusalKind::TenantShare);
                    entry
                }),
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
        // The tenant slot belongs to the sandbox once there is one, and to this request until
        // then. Held for the rest of the request so that however it ends - including being
        // dropped because the client reset the stream - the slot goes back.
        let mut reservation = Reservation::new(std::sync::Arc::clone(&self.sandboxes), id.clone());
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
            // Nothing else holds this record until the launch settles it, so the reservation
            // stays armed: if this request is dropped mid-launch, its slot goes back.
            self.create_and_wait(principal, &prepared, timing).await
        } else {
            // The answer hands the record to the launch, which owns its lifecycle from here.
            self.create_before_ready(principal, &prepared, timing, &mut reservation)
                .await
        }
    }

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
    async fn create_and_wait(
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
