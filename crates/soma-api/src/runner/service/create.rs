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

mod body;
mod early;
mod failure;
mod wait;

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
