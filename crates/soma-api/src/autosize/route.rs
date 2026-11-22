//! `GET` and `PUT /v1/sandboxes/{id}/autosize`: a sandbox's sizing range, what it uses now, and
//! every change the controller made.

use serde::Deserialize;
use soma::InstanceId;

use super::{Policy, set, view};
use crate::envelope::{ApiError, Envelope, render};
use crate::facade::SandboxFacade;
use crate::http::response::Response;

/// What a `PUT` asks for: a range to keep the sandbox within, or `enabled: false` to stop.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutosizeBody {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub min_cpu: Option<u16>,
    #[serde(default)]
    pub max_cpu: Option<u16>,
    #[serde(default)]
    pub min_memory_mb: Option<u64>,
    #[serde(default)]
    pub max_memory_mb: Option<u64>,
}

/// Serves the route; `change` is a `PUT`.
///
/// # Errors
///
/// Returns a `400` for a range that is empty or names zero, and a `404` for a sandbox this host
/// does not hold.
pub fn serve<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    instance: &InstanceId,
    change: bool,
    body: &[u8],
) -> Result<Response, ApiError> {
    let current = facade
        .resize(instance, soma_local::ResizeRequest::default())
        .map_err(|_| {
            ApiError::new(
                404,
                "machine_not_found",
                "sandbox is not running on this host",
                false,
            )
        })?;
    if change {
        let body: AutosizeBody = serde_json::from_slice(body)
            .map_err(|_| ApiError::invalid("the request body is not a valid autosize document"))?;
        if body.enabled == Some(false) {
            set(instance, None);
        } else {
            // A range left out defaults to the whole of what the machine can do: from what it
            // booted with (memory) or one processor, up to its ceiling and its booted processors.
            let policy = Policy {
                min_cpu: body.min_cpu.unwrap_or(1),
                max_cpu: body.max_cpu.unwrap_or(current.cpu.booted),
                min_memory_mb: body.min_memory_mb.unwrap_or(current.memory.booted_mb),
                max_memory_mb: body.max_memory_mb.unwrap_or(current.memory.ceiling_mb),
            };
            if policy.min_cpu == 0
                || policy.min_cpu > policy.max_cpu
                || policy.min_memory_mb > policy.max_memory_mb
            {
                return Err(ApiError::invalid(
                    "the range is empty: each minimum must be at least 1 and at most its maximum",
                ));
            }
            set(instance, Some(policy));
        }
    }
    Ok(Response::new(
        200,
        render(&Envelope::success(
            "sandbox.autosize",
            &view(instance),
            None,
        )),
    ))
}
