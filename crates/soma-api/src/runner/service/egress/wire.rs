//! The wire side of an egress window: what a caller may ask for and what it is told.

use std::time::{Duration, Instant, SystemTime};

use serde_json::{Value, json};
use soma_local::EgressSetFailure;

use crate::runner::{
    clock::iso8601_micros,
    ids::SandboxId,
    public_wire::PlatformError,
    windows::{MAX_WINDOW, Window, valid_window_id},
};

/// A window request, after validation.
pub(super) struct OpenRequest {
    pub(super) window_id: String,
    pub(super) ttl: Duration,
    pub(super) dns: bool,
}

pub(super) fn parse_open(body: &[u8]) -> Result<OpenRequest, PlatformError> {
    let invalid = |field: &str, detail: &str| PlatformError::invalid_param(field, detail);
    let document: Value = serde_json::from_slice(body)
        .map_err(|_| invalid("body", "the body must be a JSON object"))?;
    let Value::Object(fields) = document else {
        return Err(invalid("body", "the body must be a JSON object"));
    };
    if let Some(unknown) = fields
        .keys()
        .find(|key| !matches!(key.as_str(), "egress" | "ttl_ms" | "window_id" | "dns"))
    {
        return Err(invalid(unknown, "unknown field"));
    }
    if fields.get("egress").and_then(Value::as_str) != Some("internet") {
        return Err(invalid("egress", "egress must be \"internet\""));
    }
    let window_id = fields
        .get("window_id")
        .and_then(Value::as_str)
        .filter(|id| valid_window_id(id))
        .ok_or_else(|| {
            invalid(
                "window_id",
                "window_id must be 1 to 64 letters, digits, '-', '_' or '.'",
            )
        })?
        .to_owned();
    let ttl = fields
        .get("ttl_ms")
        .and_then(Value::as_u64)
        .map(Duration::from_millis)
        .filter(|ttl| !ttl.is_zero() && *ttl <= MAX_WINDOW)
        .ok_or_else(|| {
            invalid(
                "ttl_ms",
                "ttl_ms must be between 1 and 900000 (fifteen minutes)",
            )
        })?;
    let dns = match fields.get("dns") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(dns)) => *dns,
        Some(_) => return Err(invalid("dns", "dns must be a boolean")),
    };
    Ok(OpenRequest {
        window_id,
        ttl,
        dns,
    })
}

pub(super) fn window_body(id: &SandboxId, window: &Window, changed: bool) -> Vec<u8> {
    let expires_at = iso8601_micros(
        SystemTime::now() + window.expires.saturating_duration_since(Instant::now()),
    );
    json_body(&json!({
        "sandbox_id": id.as_str(),
        "window_id": window.id,
        "egress": "internet",
        "dns": window.dns,
        "changed": changed,
        "expires_at": expires_at,
    }))
}

pub(super) fn json_body(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_default()
}

pub(super) fn not_windowed() -> PlatformError {
    PlatformError::new(
        409,
        "egress_not_windowed",
        "this sandbox was not created with network.windowed, so its egress cannot be opened",
        false,
    )
}

pub(super) fn flip_failed() -> PlatformError {
    PlatformError::new(
        502,
        "egress_flip_failed",
        "the network rules could not be put in force; treat egress as unchanged for an open and as possibly open for a close",
        false,
    )
}

pub(super) fn failure_error(failure: EgressSetFailure) -> PlatformError {
    match failure {
        EgressSetFailure::NotFound => PlatformError::sandbox_not_found(),
        EgressSetFailure::NotWindowed => not_windowed(),
        EgressSetFailure::NotActive => PlatformError::new(
            409,
            "egress_not_active",
            "the sandbox's network is not active yet",
            true,
        ),
        EgressSetFailure::FlipFailed => flip_failed(),
        EgressSetFailure::Unavailable => PlatformError::new(
            503,
            "egress_unavailable",
            "the sandbox's host could not be reached to change its network",
            true,
        ),
    }
}

#[cfg(test)]
mod tests;
