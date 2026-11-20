//! The platform's file routes on a sandbox this runner holds.
//!
//! A file call is addressed, waited for, and held exactly as a forwarded inspect or terminal
//! call is, so it reaches a sandbox only when every other per-sandbox route would. Its arguments
//! are then checked as the platform checks them, and the whole call runs on one facade lease: a
//! write that is several guest operations is one unit of work to the runner, which is what keeps
//! its temporary sibling from outliving it.

use crate::runner::{
    files::{self, Answer, FileCall, FileError, OnFacade, Params, ParamsRefused, Plan, Stop},
    keys::Principal,
    public_wire::PlatformError,
};

use super::{
    MAX_BODY_BYTES, Runner, RunnerRequest, RunnerResponse, Timing,
    forward::unavailable,
    outcome,
    routing::{Route, route},
};

impl Runner {
    /// The most body bytes the transport reads for this request before handing it on.
    ///
    /// Every route takes the loopback service's bound, except a JSON file write from a caller
    /// whose key is admitted: that one carries a file of up to the route's own bound as base64.
    /// The key is checked before the larger allowance is granted, because the body is read
    /// before the request is authenticated, and a bound anyone could claim would let anyone make
    /// this process buffer it.
    #[must_use]
    pub fn body_limit(
        &self,
        method: &http::Method,
        path: &str,
        authorization: Option<&str>,
    ) -> usize {
        match route(method, path) {
            Route::Files(_, FileCall::Write) if self.keys.admit(authorization).is_ok() => {
                files::MAX_JSON_WRITE_BODY_BYTES
            }
            _ => MAX_BODY_BYTES,
        }
    }

    pub(super) async fn files(
        &self,
        principal: &Principal,
        raw_id: &str,
        call: FileCall,
        request: &RunnerRequest,
        timing: &mut Timing,
    ) -> RunnerResponse {
        let id = match self.addressed(raw_id) {
            Ok(id) => id,
            Err(response) => return *response,
        };
        let waited = match self.wait_for_launch(&id, &principal.key.tenant_id).await {
            Ok(waited) => waited,
            Err(response) => return *response,
        };
        timing.wait = waited;
        // Held for the whole call, so a sandbox is never reaped under a write in progress.
        let _held = match self.sandboxes.hold(&id, &principal.key.tenant_id) {
            Ok(held) => held,
            Err(refused) => return unavailable(&refused),
        };
        let Some(instance_id) = id.instance_id() else {
            return RunnerResponse::platform(&PlatformError::sandbox_not_found());
        };
        let plan = match plan(call, request) {
            Ok(plan) => plan,
            Err(error) => return refused(&error),
        };
        let performed = self
            .backend
            .call(move |facade| {
                plan.run(&mut OnFacade {
                    facade,
                    instance_id: &instance_id,
                })
            })
            .await;
        match performed {
            Ok((outcome, call_timing)) => {
                timing.call = call_timing;
                answer(call, outcome)
            }
            Err(_busy) => RunnerResponse::runtime_busy(),
        }
    }
}

/// The call's checked arguments, from its query string and body.
fn plan(call: FileCall, request: &RunnerRequest) -> Result<Plan, FileError> {
    let params =
        Params::read(request.query.as_deref(), &request.body).map_err(|refused| match refused {
            ParamsRefused::Query => FileError::invalid_query(),
            ParamsRefused::Body => FileError::invalid_body("the body is not one JSON object"),
        })?;
    Plan::of(call, &params)
}

fn answer(call: FileCall, outcome: Result<Answer, Stop>) -> RunnerResponse {
    match outcome {
        Ok(Answer { status, body }) => RunnerResponse::new(status, body),
        Err(Stop::Refused(refusal)) => refused(&files::refusal(call, refusal)),
        // The sandbox itself could not be reached: the same answer every per-sandbox route gives.
        Err(Stop::Failed(failure)) => RunnerResponse::platform(&outcome::failure_error(&failure)),
        Err(Stop::Unexpected) => {
            eprintln!(
                "soma-api: runner file call {} got an answer it never gets",
                call.name()
            );
            RunnerResponse::platform(&PlatformError::agent_unavailable())
        }
    }
}

fn refused(error: &FileError) -> RunnerResponse {
    RunnerResponse::new(error.status, error.body())
}
