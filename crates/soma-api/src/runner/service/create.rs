use std::time::{Duration, Instant, SystemTime};

use soma::{LaunchMachineRequest, MachineName};

use crate::{
    runner::{
        clock::iso8601_micros, ids::SandboxId, journal::EntryKind, keys::Principal, public_wire,
        sandboxes::Owner,
    },
    wire::operation_id,
};

use super::{
    Runner, RunnerResponse, TENANT_LABEL_PREFIX, Timing, entry, failure_code, params::CreateParams,
    with_journal,
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
                RunnerResponse::retry_elsewhere("runtime_busy"),
                journal(503, None, project),
            );
        };
        let id = SandboxId::mint(self.config.host_tag);
        let owner = Owner {
            tenant_id: principal.key.tenant_id.clone(),
            key_id: Some(principal.key.key_id.clone()),
            project_id: project.clone(),
            created: Instant::now(),
        };
        let lifetime = Duration::from_secs(params.timeout_seconds);
        if !self
            .sandboxes
            .reserve(id.clone(), owner, lifetime, principal.policy.max_concurrent)
        {
            return with_journal(
                RunnerResponse::refusal(429, "rate_limited"),
                journal(429, None, project),
            );
        }
        let Some(launch) = self.launch_request(&id, &principal.key.tenant_id) else {
            self.sandboxes.abandon(&id);
            return with_journal(create_unavailable(), journal(503, Some(&id), project));
        };
        let outcome = self
            .backend
            .call(move |facade| {
                if facade.hosts_addressable_sandboxes() {
                    facade
                        .launch(launch)
                        .map(|_| ())
                        .map_err(|failure| failure_code(&failure))
                } else {
                    Err("durable_machine_hosting_missing")
                }
            })
            .await;
        match outcome {
            Ok((Ok(()), call)) => {
                timing.call = call;
                self.sandboxes.confirm(&id);
                with_journal(
                    RunnerResponse::new(201, self.created_body(&id, params.timeout_seconds)),
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
                    RunnerResponse::retry_elsewhere("runtime_busy"),
                    journal(503, Some(&id), project),
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
            || self.keys.feed_age(Instant::now()) > self.config.feed_stale_after()
        {
            return Err(Box::new(RunnerResponse::retry_elsewhere("feed_stale")));
        }
        let params = CreateParams::parse(
            body,
            self.config.launch.default_timeout_seconds,
            &self.config.launch.shape,
        )
        .map_err(|error| Box::new(RunnerResponse::platform(&error)))?;
        if !principal.key.projects.allows(params.project_id.as_deref()) {
            return Err(Box::new(RunnerResponse::refusal(403, "forbidden")));
        }
        Ok(params)
    }

    fn created_body(&self, id: &SandboxId, timeout_seconds: u64) -> Vec<u8> {
        let created_at = iso8601_micros(SystemTime::now());
        public_wire::encode(&public_wire::Created {
            cpu_count: self.config.launch.shape.vcpu_count(),
            created_at: &created_at,
            deletion_pending: false,
            id: id.as_str(),
            memory_mb: self.config.launch.shape.memory_mib(),
            name: None,
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
