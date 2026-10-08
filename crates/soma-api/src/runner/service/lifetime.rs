use std::time::{Duration, Instant};

use soma::{DestroyMachineRequest, ManagedFailure, ManagedStateError, SandboxPhase};

use crate::{
    runner::{
        backend::CallTiming,
        idle::Lifetime,
        ids::SandboxId,
        journal::{Entry, EntryKind},
        keys::Principal,
        public_wire::{self, PlatformError},
        sandboxes::{Owner, Unavailable},
    },
    wire::operation_id,
};

use super::{
    Runner, RunnerResponse, TENANT_LABEL_PREFIX, Timing, entry, failure_code, millis,
    refused_sandbox, with_journal,
};

impl Runner {
    pub(super) async fn destroy(
        &self,
        principal: &Principal,
        raw_id: &str,
        timing: &mut Timing,
    ) -> RunnerResponse {
        let id = match self.addressed(raw_id) {
            Ok(id) => id,
            Err(response) => return refused_sandbox(*response, EntryKind::Destroy, principal),
        };
        let journal = |status: u16, owner: Option<&Owner>, lifetime: Option<Duration>| {
            let mut entry = entry(
                EntryKind::Destroy,
                principal,
                owner.and_then(|owner| owner.project_id.clone()),
                Some(&id),
                status,
            );
            entry.lifetime_ms = lifetime.map(millis);
            Some(entry)
        };
        let owner = match self.sandboxes.begin_destroy(&id, &principal.key.tenant_id) {
            Ok(owner) => owner,
            Err(Unavailable::Destroyed(_, lifetime)) => {
                // A repeated destroy answers as the first one did, and is not paperwork again.
                return RunnerResponse::new(200, destroyed_body(&id, lifetime));
            }
            Err(Unavailable::NotFound) => {
                let error = PlatformError::sandbox_not_found();
                return with_journal(RunnerResponse::platform(&error), journal(404, None, None));
            }
            Err(Unavailable::Busy) => {
                let error = PlatformError::destroy_busy();
                return with_journal(RunnerResponse::platform(&error), journal(409, None, None));
            }
        };
        let result = self.release(&id).await;
        if let Some(call) = result.timing {
            timing.call = call;
        }
        match result.outcome {
            Released::Gone(at) => {
                let lifetime = at.saturating_duration_since(owner.created);
                with_journal(
                    RunnerResponse::new(200, destroyed_body(&id, lifetime)),
                    journal(200, Some(&owner), Some(lifetime)),
                )
            }
            Released::Failed(code) => {
                let error = PlatformError::destroy_outcome_unknown(code);
                with_journal(
                    RunnerResponse::platform(&error),
                    journal(503, Some(&owner), None),
                )
            }
            Released::Busy => with_journal(
                RunnerResponse::runtime_busy(),
                journal(429, Some(&owner), None),
            ),
        }
    }

    /// Destroys one sandbox the caller has already claimed with `begin_destroy` or the reaper.
    async fn release(&self, id: &SandboxId) -> Release {
        let Some(instance_id) = id.instance_id() else {
            self.sandboxes.release(id);
            return Release {
                outcome: Released::Failed("invalid_instance_id"),
                timing: None,
            };
        };
        let Ok(operation) = operation_id(None) else {
            self.sandboxes.release(id);
            return Release {
                outcome: Released::Failed("operation_id_rejected"),
                timing: None,
            };
        };
        let request = DestroyMachineRequest::new(operation, instance_id);
        match self
            .backend
            .call(move |facade| facade.destroy(request))
            .await
        {
            // A sandbox the state store does not know is already gone, which is what was asked.
            Ok((Ok(_) | Err(ManagedFailure::State(ManagedStateError::MachineNotFound)), call)) => {
                let at = Instant::now();
                self.sandboxes.destroyed(id, at);
                Release {
                    outcome: Released::Gone(at),
                    timing: Some(call),
                }
            }
            Ok((Err(failure), call)) => {
                self.sandboxes.release(id);
                Release {
                    outcome: Released::Failed(failure_code(&failure)),
                    timing: Some(call),
                }
            }
            Err(_busy) => {
                self.sandboxes.release(id);
                Release {
                    outcome: Released::Busy,
                    timing: None,
                }
            }
        }
    }

    /// One sweep of contract C7: destroys every sandbox whose lifetime has run out or whose
    /// tenant is suspended, and forgets old destroyed ones.
    ///
    /// The fast lane's sandboxes expired on the control plane's schedule; a runner sandbox has
    /// no one else to end it, so the runner does, and journals it as `expire`.
    /// The control plane's own reaper stays as a backstop.
    pub async fn reap(&self) {
        let now = Instant::now();
        self.sandboxes.sweep(now);
        let keys = &self.keys;
        let claimed = self
            .sandboxes
            .claim_expired(now, |tenant| keys.reap_reason(tenant));
        for (id, owner, reason) in claimed {
            let result = self.release(&id).await;
            if let Released::Gone(at) = result.outcome {
                self.journal.record(Entry {
                    kind: EntryKind::Expire,
                    tenant_id: owner.tenant_id.clone(),
                    key_id: owner.key_id.clone(),
                    project_id: owner.project_id.clone(),
                    sandbox_id: Some(id.to_string()),
                    status: 200,
                    ms: result
                        .timing
                        .map_or(0, |call| millis(call.pool + call.exec)),
                    exit_code: None,
                    cpu_ms: None,
                    lifetime_ms: Some(millis(at.saturating_duration_since(owner.created))),
                    reason: Some(reason),
                });
            }
        }
    }

    /// Re-adopts the sandboxes the state store still holds, so a restarted runner keeps serving
    /// them to the tenants that created them.
    ///
    /// Ownership comes from the tenant label the runner writes at create. A sandbox without one
    /// was not created here and stays unreachable through the runner. The original creation time
    /// is not in the listing, so a recovered sandbox gets a fresh default lifetime.
    pub async fn recover_sandboxes(&self) {
        let entries = match self.backend.call(|facade| facade.list()).await {
            Ok((Ok(entries), _)) => entries,
            Ok((Err(failure), _)) => {
                eprintln!(
                    "soma-api: runner could not list sandboxes to recover their owners: {}",
                    failure_code(&failure)
                );
                return;
            }
            Err(_busy) => {
                eprintln!("soma-api: runner could not lease a facade to recover sandbox owners");
                return;
            }
        };
        // The tenant's own default and cap are not known before the feed arrives, so a
        // recovered sandbox gets this runner's default idle timeout.
        let lifetime = Lifetime::from_seconds(self.config.launch.default_timeout_seconds, None);
        let mut recovered = 0_usize;
        for entry in entries {
            if !matches!(
                entry.phase(),
                SandboxPhase::Active | SandboxPhase::Executing
            ) {
                continue;
            }
            let (Some(id), Some(tenant)) = (
                SandboxId::from_instance_id(entry.instance_id()),
                entry
                    .name()
                    .and_then(|name| name.as_str().strip_prefix(TENANT_LABEL_PREFIX)),
            ) else {
                continue;
            };
            let owner = Owner {
                tenant_id: tenant.to_owned(),
                key_id: None,
                project_id: None,
                created: Instant::now(),
                slot: self.keys.counter(tenant),
            };
            self.sandboxes.recover(id, owner, lifetime);
            recovered += 1;
        }
        eprintln!("soma-api: runner recovered {recovered} sandboxes");
    }
}

struct Release {
    outcome: Released,
    timing: Option<CallTiming>,
}

enum Released {
    /// Destroyed at this instant.
    Gone(Instant),
    Failed(&'static str),
    Busy,
}

fn destroyed_body(id: &SandboxId, lifetime: Duration) -> Vec<u8> {
    public_wire::encode(&public_wire::Destroyed {
        // `cpu_ms` and `mem_peak_bytes` stay null because the facade and the receipt it returns
        // carry no per-sandbox CPU or peak-memory figure today (contract C7). `lifetime_ms` is
        // the sandbox's uptime, so it is not repeated under a second name.
        cpu_ms: None,
        id: id.as_str(),
        lifetime_ms: millis(lifetime),
        mem_peak_bytes: None,
        operation_id: None,
        state: "destroyed",
        total_runtime_sec: None,
    })
}
