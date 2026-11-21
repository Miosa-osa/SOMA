use std::{sync::Arc, time::Instant};

use soma::{LaunchMachineRequest, MachineName, MachineShape, OciImage, OperationId};

use crate::{
    handler::checkpoint,
    runner::{
        idle::Lifetime,
        ids::SandboxId,
        keys::Principal,
        public_wire::PlatformError,
        sandboxes::{Held, Owner, Reservation, Unavailable},
    },
};

use super::{Runner, RunnerResponse, TENANT_LABEL_PREFIX, Timing, forward};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ForkBody {
    operation_id: OperationId,
    checkpoint_id: OperationId,
    image: String,
    shape: MachineShape,
}

enum ForkDestination<'a> {
    Existing(Held<'a>),
    New(Reservation),
}

impl Runner {
    pub(super) async fn fork(
        &self,
        principal: &Principal,
        raw_source: &str,
        body: &[u8],
        timing: &mut Timing,
    ) -> RunnerResponse {
        let id = match self.addressed(raw_source) {
            Ok(id) => id,
            Err(response) => return *response,
        };
        let Some(source) = id.instance_id() else {
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
        let tenant = &principal.key.tenant_id;
        let Ok(owner) = MachineName::parse(format!("{TENANT_LABEL_PREFIX}{tenant}")) else {
            return not_found();
        };
        let probe = source.clone();
        match self
            .backend
            .call(move |facade| facade.restore_owner(&probe))
            .await
        {
            Ok((Ok(Some(label)), _)) if label == owner => {}
            Ok((Ok(_), _)) => return not_found(),
            Ok((Err(_), _)) | Err(_) => return RunnerResponse::runtime_busy(),
        }
        let Ok(body) = serde_json::from_slice::<ForkBody>(body) else {
            return RunnerResponse::platform(&PlatformError::invalid_param(
                "body",
                "expected operation_id, checkpoint_id, image, and shape",
            ));
        };
        let Ok(image) = OciImage::parse(body.image) else {
            return RunnerResponse::platform(&PlatformError::invalid_param(
                "image",
                "expected a valid OCI image reference",
            ));
        };
        let destination = id.fork(tenant, &body.operation_id);
        let Some(instance) = destination.instance_id() else {
            return RunnerResponse::runtime_busy();
        };
        let (_held, reservation) = match self.reserve_fork(principal, &destination) {
            Ok(ForkDestination::Existing(held)) => (Some(held), None),
            Ok(ForkDestination::New(reservation)) => (None, Some(reservation)),
            Err(response) => return *response,
        };
        let registry = Arc::clone(&self.sandboxes);
        let launch = LaunchMachineRequest::new(body.operation_id, instance, image, body.shape)
            .with_name(owner);
        let result = self
            .backend
            .call(move |facade| {
                let response = checkpoint::fork_as(
                    facade,
                    launch,
                    &source,
                    &body.checkpoint_id,
                    destination.as_str(),
                );
                if response.status == 200
                    && let Some(mut reservation) = reservation
                {
                    registry.confirm(&destination);
                    reservation.disarm();
                }
                response
            })
            .await;
        match result {
            Ok((response, call)) => {
                timing.call = call;
                RunnerResponse::new(response.status, response.body)
            }
            Err(_) => RunnerResponse::runtime_busy(),
        }
    }

    fn reserve_fork(
        &self,
        principal: &Principal,
        destination: &SandboxId,
    ) -> Result<ForkDestination<'_>, Box<RunnerResponse>> {
        let tenant = &principal.key.tenant_id;
        match self.sandboxes.hold(destination, tenant) {
            Ok(held) => return Ok(ForkDestination::Existing(held)),
            Err(Unavailable::NotFound) => {}
            Err(error) => return Err(Box::new(forward::unavailable(&error))),
        }
        if !principal.tenant.admit() {
            return Err(Box::new(RunnerResponse::refusal(429, "rate_limited")));
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
            .reserve_restore(destination.clone(), owner.clone(), lifetime)
        {
            owner.give_back_slot();
            return Err(Box::new(forward::unavailable(&Unavailable::Busy)));
        }
        Ok(ForkDestination::New(Reservation::new(
            Arc::clone(&self.sandboxes),
            destination.clone(),
        )))
    }
}

fn not_found() -> RunnerResponse {
    RunnerResponse::platform(&PlatformError::sandbox_not_found())
}
