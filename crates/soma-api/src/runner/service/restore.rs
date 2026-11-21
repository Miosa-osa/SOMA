//! Same-Instance checkpoint restore through the authenticated runner.

use std::{sync::Arc, time::Instant};

use soma::MachineName;

use crate::{
    handler::checkpoint,
    runner::{
        idle::Lifetime,
        keys::Principal,
        public_wire::PlatformError,
        sandboxes::{Owner, Reservation, Unavailable},
    },
};

use super::{Runner, RunnerResponse, TENANT_LABEL_PREFIX, Timing, forward};

impl Runner {
    pub(super) async fn restore(
        &self,
        principal: &Principal,
        raw_id: &str,
        body: &[u8],
        timing: &mut Timing,
    ) -> RunnerResponse {
        let id = match self.addressed(raw_id) {
            Ok(id) => id,
            Err(response) => return *response,
        };
        let Some(instance) = id.instance_id() else {
            return not_found();
        };
        let tenant = &principal.key.tenant_id;
        let Ok(label) = MachineName::parse(format!("{TENANT_LABEL_PREFIX}{tenant}")) else {
            return not_found();
        };
        if !self.keys.has_received()
            || self.keys.resync_required()
            || self.keys.feed_age(Instant::now()) > self.config.feed_stale_after()
        {
            return RunnerResponse::feed_stale();
        }
        let Ok(_permit) = self.admission.try_acquire() else {
            return RunnerResponse::runtime_busy();
        };
        timing.wait = match self.wait_for_launch(&id, tenant).await {
            Ok(waited) => waited,
            Err(response) => return *response,
        };
        let mut reservation = None;
        let held = match self.sandboxes.hold(&id, tenant) {
            Ok(held) => Some(held),
            Err(Unavailable::NotFound) => {
                let probe = instance.clone();
                match self
                    .backend
                    .call(move |facade| facade.restore_owner(&probe))
                    .await
                {
                    Ok((Ok(Some(owner)), _)) if owner == label => {}
                    Ok((Ok(_), _)) => return not_found(),
                    Ok((Err(_), _)) | Err(_) => return RunnerResponse::runtime_busy(),
                }
                if !principal.tenant.admit() {
                    return RunnerResponse::refusal(429, "rate_limited");
                }
                let owner = Owner {
                    tenant_id: tenant.clone(),
                    key_id: Some(principal.key.key_id.clone()),
                    project_id: None,
                    created: Instant::now(),
                    slot: principal.tenant.counter(),
                };
                let lifetime = Lifetime::from_seconds(
                    principal
                        .tenant
                        .policy
                        .default_timeout_seconds
                        .unwrap_or(self.config.launch.default_timeout_seconds),
                    principal.tenant.policy.max_lifetime_seconds,
                );
                if !self
                    .sandboxes
                    .reserve_restore(id.clone(), owner.clone(), lifetime)
                {
                    owner.give_back_slot();
                    return forward::unavailable(&Unavailable::Busy);
                }
                reservation = Some(Reservation::new(Arc::clone(&self.sandboxes), id.clone()));
                None
            }
            Err(error) => return forward::unavailable(&error),
        };
        let registry = Arc::clone(&self.sandboxes);
        let body = body.to_vec();
        let result = self
            .backend
            .call(move |facade| {
                let response = checkpoint::restore_as(facade, instance, &body, label);
                if response.status == 200
                    && let Some(mut reserved) = reservation
                {
                    registry.confirm(&id);
                    reserved.disarm();
                }
                response
            })
            .await;
        drop(held);
        match result {
            Ok((response, call)) => {
                timing.call = call;
                RunnerResponse::new(response.status, response.body)
            }
            Err(_) => RunnerResponse::runtime_busy(),
        }
    }
}

fn not_found() -> RunnerResponse {
    RunnerResponse::platform(&PlatformError::sandbox_not_found())
}
