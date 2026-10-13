//! The prewarm routes: what is prepared on this host, read and changed (ADR 0052).
//!
//! Served by the connection before a sandbox facade is opened, so a burst that holds every facade
//! cannot also block a change to what is prepared.

use serde::Serialize;

use super::{
    ApiError, Request, Response, Route, TENANT_HEADER, failure_body_with_result, identify, parse,
    refuse, resolve, success,
};

/// Serves a prewarm request without a sandbox facade.
///
/// Identity is established first, as in [`crate::handler::handle`]. The connection calls this before it opens a
/// facade, so a burst that holds every facade cannot also block a change to what is prepared.
#[must_use]
pub fn handle_prewarm(request: &Request) -> Response {
    if let Err(error) = identify(request.header(TENANT_HEADER)) {
        return refuse("request", error);
    }
    match resolve(request) {
        Ok(Route::Prewarm(write)) => {
            let operation = Route::Prewarm(write).operation();
            prewarm_response(operation, write, &request.body)
        }
        Ok(_) => refuse("request", ApiError::not_found()),
        Err(error) => refuse("request", error),
    }
}

/// The body of `PUT /v1/prewarm`: the whole desired set.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PrewarmSetBody {
    targets: Vec<soma_local::PrewarmTarget>,
}

#[derive(Serialize)]
struct PrewarmRefusal {
    #[serde(skip_serializing_if = "Option::is_none")]
    reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    budget_mib: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    requested_mib: Option<u64>,
}

fn prewarm_response(operation: &'static str, write: bool, body: &[u8]) -> Response {
    if !write {
        return success(200, operation, &soma_local::prewarm_status(), None);
    }
    let set = match parse::<PrewarmSetBody>(body) {
        Ok(set) => set,
        Err(error) => return refuse(operation, error),
    };
    match soma_local::set_prewarm_targets(&set.targets) {
        Ok(status) => success(200, operation, &status, None),
        Err(error) => prewarm_refusal(operation, error),
    }
}

/// Names a refused set, with the facts the caller needs to correct it.
fn prewarm_refusal(operation: &'static str, error: soma_local::PrewarmError) -> Response {
    use soma_local::PrewarmError as E;
    let nothing = PrewarmRefusal {
        reference: None,
        reason: None,
        budget_mib: None,
        requested_mib: None,
    };
    let (api, detail) = match error {
        E::UnknownReference(reference) => (
            ApiError::new(
                422,
                "unknown_reference",
                "no Generation is prepared for this reference on this host",
                false,
            ),
            PrewarmRefusal {
                reference: Some(reference),
                ..nothing
            },
        ),
        E::InvalidTarget(reason) => (
            ApiError::new(422, "invalid_target", "the prewarm set is not valid", false),
            PrewarmRefusal {
                reason: Some(reason),
                ..nothing
            },
        ),
        E::BudgetExceeded {
            budget_mib,
            requested_mib,
        } => (
            ApiError::new(
                422,
                "prewarm_budget_exceeded",
                "the prewarm set asks for more memory than this host's budget",
                false,
            ),
            PrewarmRefusal {
                budget_mib: Some(budget_mib),
                requested_mib: Some(requested_mib),
                ..nothing
            },
        ),
        E::Unsupported => (
            ApiError::new(
                501,
                "prewarm_unsupported",
                "this host does not prepare machines",
                false,
            ),
            nothing,
        ),
        E::StoreUnavailable(reason) => (
            ApiError::new(
                503,
                "prewarm_store_unavailable",
                "the prepared store on this host cannot be used",
                false,
            ),
            PrewarmRefusal {
                reason: Some(reason),
                ..nothing
            },
        ),
        E::Unpersisted(reason) => (
            ApiError::new(
                500,
                "prewarm_not_persisted",
                "the prewarm set was valid but could not be saved, so nothing changed",
                false,
            ),
            PrewarmRefusal {
                reason: Some(reason),
                ..nothing
            },
        ),
    };
    Response::new(
        api.status(),
        failure_body_with_result(operation, api, &detail),
    )
}
