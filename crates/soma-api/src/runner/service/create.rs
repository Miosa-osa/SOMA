use std::time::{Instant, SystemTime};

use soma::{LaunchMachineRequest, MachineName};

use crate::{
    runner::{
        clock::iso8601_micros, idle::Lifetime, ids::SandboxId, journal::EntryKind, keys::Principal,
        public_wire, sandboxes::Owner,
    },
    wire::operation_id,
};

use super::{
    Runner, RunnerResponse, TENANT_LABEL_PREFIX, Timing, entry, failure_code, millis,
    params::CreateParams, timing::LaunchPhases, with_journal,
};

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
        let Some(launch) = self.launch_request(&id, &principal.key.tenant_id) else {
            self.sandboxes.abandon(&id);
            return with_journal(create_unavailable(), journal(503, Some(&id), project));
        };
        timing.prep = prep_started.elapsed();
        let outcome = self
            .backend
            .call(move |facade| {
                if facade.hosts_addressable_sandboxes() {
                    // The receipt comes back so the answer can report the phases
                    // the launch evidence already timed; the runner itself does
                    // not read it for anything else.
                    facade
                        .launch(launch)
                        .map(|launched| launched.receipt)
                        .map_err(|failure| failure_code(&failure))
                } else {
                    Err("durable_machine_hosting_missing")
                }
            })
            .await;
        match outcome {
            Ok((Ok(receipt), call)) => {
                timing.call = call;
                timing.launch = LaunchPhases::from_receipt(&receipt, call.exec);
                let finish_started = Instant::now();
                self.sandboxes.confirm(&id);
                let runner_url = self.runner_url(self.config.host_tag);
                let body = self.created_body(
                    &id,
                    &runner_url,
                    params.timeout_seconds,
                    millis(call.pool + call.exec),
                );
                timing.finish = finish_started.elapsed();
                with_journal(
                    RunnerResponse::new(201, body).header("soma-runner-url", runner_url),
                    journal(201, Some(&id), project),
                )
            }
            Ok((Err(code), call)) => {
                timing.call = call;
                self.sandboxes.abandon(&id);
                eprintln!("soma-api: runner create {id} failed: {code}");
                with_journal(create_unavailable(), journal(503, Some(&id), project))
            }
            Err(_busy) => {
                self.sandboxes.abandon(&id);
                with_journal(
                    RunnerResponse::runtime_busy(),
                    journal(429, Some(&id), project),
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
        let params = CreateParams::parse(body, default_timeout, &self.config.launch.shape)
            .map_err(|error| Box::new(RunnerResponse::platform(&error)))?;
        if !principal.key.projects.allows(params.project_id.as_deref()) {
            return Err(Box::new(RunnerResponse::refusal(403, "forbidden")));
        }
        Ok(params)
    }

    fn created_body(
        &self,
        id: &SandboxId,
        runner_url: &str,
        timeout_seconds: u64,
        create_ms: u64,
    ) -> Vec<u8> {
        let created_at = iso8601_micros(SystemTime::now());
        public_wire::encode(&public_wire::Created {
            cpu_count: self.config.launch.shape.vcpu_count(),
            create_ms,
            created_at: &created_at,
            deletion_pending: false,
            id: id.as_str(),
            memory_mb: self.config.launch.shape.memory_mib(),
            name: None,
            runner_url,
            slug: &id.as_str()[..8],
            state: "running",
            template_id: &self.config.launch.template_id,
            timeout_sec: timeout_seconds,
        })
    }

    fn launch_request(&self, id: &SandboxId, tenant_id: &str) -> Option<LaunchMachineRequest> {
        let instance_id = id.instance_id()?;
        let operation = operation_id(None).ok()?;
        let request = LaunchMachineRequest::new(
            operation,
            instance_id,
            self.image.clone(),
            self.config.launch.shape.clone(),
        );
        Some(
            match MachineName::parse(format!("{TENANT_LABEL_PREFIX}{tenant_id}")) {
                Ok(name) => request.with_name(name),
                Err(_) => request,
            },
        )
    }
}

fn create_unavailable() -> RunnerResponse {
    RunnerResponse::new(503, public_wire::create_unavailable()).header("retry-after", "1".into())
}
