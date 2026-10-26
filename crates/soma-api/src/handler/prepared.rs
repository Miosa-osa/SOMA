//! The prepared-store route: plan, and when asked apply, a prune (ADR 0054).
//!
//! Served with a facade, because what still uses a Generation is what the durable store's records
//! say, and the facade is what reads them. The facade is asked twice on an applied prune: once to
//! plan and once after the entries are out of the root, so a launch that recorded itself in
//! between keeps its entry.

use serde::Serialize;
use soma_local::{InstanceHolds, PruneError, PruneRequest};

use super::{ApiError, Response, failure_body_with_result, optional_body, success};
use crate::{facade::SandboxFacade, failure::managed_error};

/// Plans a prune and, when the body says `apply`, performs it.
///
/// An absent body is a dry run with nothing kept beyond what this host pins itself.
pub(super) fn prune<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    operation: &'static str,
    body: &[u8],
) -> Result<Response, ApiError> {
    let request = optional_body::<PruneRequest>(body)?;
    let mut instances = || {
        facade
            .list()
            .map(|listing| InstanceHolds::from_listing(&listing))
            .map_err(|failure| managed_error(&failure).body().code.to_owned())
    };
    Ok(match soma_local::prune_prepared(&request, &mut instances) {
        Ok(plan) => success(200, operation, &plan, None),
        Err(error) => refusal(operation, error),
    })
}

#[derive(Serialize)]
struct PruneRefusal {
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

/// Names a refused prune, with the reason the caller needs to correct it.
fn refusal(operation: &'static str, error: PruneError) -> Response {
    let code = error.code();
    let (api, reason) = match error {
        PruneError::Unsupported => (
            ApiError::new(501, code, "this host has no prepared store", false),
            None,
        ),
        PruneError::StoreUnavailable(reason) => (
            ApiError::new(
                503,
                code,
                "the prepared store on this host cannot be read",
                false,
            ),
            Some(reason),
        ),
        PruneError::KeepRequired => (
            ApiError::new(
                422,
                code,
                "an applied prune must name every reference the caller still needs in keep",
                false,
            ),
            None,
        ),
        PruneError::InvalidKeep(reason) => (
            ApiError::new(422, code, "the keep list is not valid", false),
            Some(reason),
        ),
        PruneError::InstancesUnavailable(reason) => (
            ApiError::new(
                503,
                code,
                "what holds each Generation could not be read, so nothing can be shown unused",
                true,
            ),
            Some(reason),
        ),
        PruneError::Busy => (
            ApiError::new(409, code, "another prune is running on this host", true),
            None,
        ),
    };
    Response::new(
        api.status(),
        failure_body_with_result(operation, api, &PruneRefusal { reason }),
    )
}
