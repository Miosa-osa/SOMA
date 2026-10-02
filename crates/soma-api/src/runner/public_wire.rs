//! The public response bodies, byte-compatible with the fast lane they replace.
//!
//! Contract C2 keeps every body the fast-lane listener (`Web.SomaFastLaneEdge.Listener` and
//! `Web.SomaFastLane.Responder`, miosa-compute 71c9430633) returns. Those are Elixir maps
//! rendered by Jason, and a small Elixir map renders its keys in sorted order, so every struct
//! here declares its fields alphabetically; `serde_json` then writes the same bytes. The
//! runner's own refusals (401, 403, 421, 429, 503) are the short bodies C2 defines for them.

use serde::Serialize;
use serde_json::Value;

/// The compact `201` body `SandboxView.render_compact/1` produces.
#[derive(Serialize)]
pub struct Created<'a> {
    pub cpu_count: u16,
    pub created_at: &'a str,
    pub deletion_pending: bool,
    pub id: &'a str,
    pub memory_mb: u64,
    pub name: Option<&'a str>,
    pub slug: &'a str,
    pub state: &'a str,
    pub template_id: &'a str,
    pub timeout_sec: u64,
}

/// The `200` exec body: `%{data: %{stdout, stderr, exit_code}}`.
#[derive(Serialize)]
pub struct Executed<'a> {
    pub data: ExecutedData<'a>,
}

#[derive(Serialize)]
pub struct ExecutedData<'a> {
    pub exit_code: i32,
    pub stderr: &'a str,
    pub stdout: &'a str,
}

/// The `200` destroy body `Responder.destroy_result/2` produces.
#[derive(Serialize)]
pub struct Destroyed<'a> {
    pub id: &'a str,
    pub operation_id: Option<&'a str>,
    pub state: &'a str,
    pub total_runtime_sec: Option<u64>,
}

/// `Web.ApiError.body/3`: `%{ok: false, error: %{code, message, retryable, ...}}`.
#[derive(Serialize)]
struct ApiErrorBody<'a> {
    error: ApiErrorFields<'a>,
    ok: bool,
}

#[derive(Serialize)]
struct ApiErrorFields<'a> {
    code: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<Value>,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_ms: Option<u64>,
    retryable: bool,
}

/// The create-capacity body `CreateErrors.capacity_error/5` produces.
#[derive(Serialize)]
struct CapacityBody<'a> {
    error: CapacityFields<'a>,
}

#[derive(Serialize)]
struct CapacityFields<'a> {
    code: &'a str,
    message: &'a str,
    retry_after: u64,
    retryable: bool,
}

/// One platform API error, as `Web.ApiError.send_error/5` renders it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformError {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
    pub details: Option<Value>,
    pub retry_after_ms: Option<u64>,
}

impl PlatformError {
    #[must_use]
    pub fn new(status: u16, code: &'static str, message: &str, retryable: bool) -> Self {
        Self {
            status,
            code,
            message: message.to_owned(),
            retryable,
            details: None,
            retry_after_ms: None,
        }
    }

    #[must_use]
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    #[must_use]
    pub const fn with_retry_after_ms(mut self, retry_after_ms: u64) -> Self {
        self.retry_after_ms = Some(retry_after_ms);
        self
    }

    #[must_use]
    pub fn body(&self) -> Vec<u8> {
        encode(&ApiErrorBody {
            error: ApiErrorFields {
                code: self.code,
                details: self.details.clone(),
                message: &self.message,
                retry_after_ms: self.retry_after_ms,
                retryable: self.retryable,
            },
            ok: false,
        })
    }

    #[must_use]
    pub fn sandbox_not_found() -> Self {
        Self::new(404, "NOT_FOUND", "sandbox not found", false)
    }

    #[must_use]
    pub fn invalid_id() -> Self {
        Self::new(400, "INVALID_ID", "sandbox id is not a valid UUID", false)
    }

    #[must_use]
    pub fn missing_command() -> Self {
        Self::new(
            400,
            "MISSING_PARAM",
            "missing required parameter: command",
            false,
        )
        .with_details(serde_json::json!({"field": "command"}))
    }

    #[must_use]
    pub fn invalid_timeout() -> Self {
        Self::new(
            400,
            "INVALID_TIMEOUT",
            "timeout must be an integer between 1 and 86400 seconds",
            false,
        )
    }

    #[must_use]
    pub fn sandbox_not_running() -> Self {
        Self::new(
            409,
            "SANDBOX_NOT_RUNNING",
            "sandbox must be in running state to exec",
            false,
        )
    }

    #[must_use]
    pub fn exec_busy() -> Self {
        Self::new(
            409,
            "SANDBOX_BUSY",
            "another command is executing in this sandbox; retry shortly",
            true,
        )
    }

    #[must_use]
    pub fn destroy_busy() -> Self {
        Self::new(
            409,
            "SANDBOX_BUSY",
            "Another lifecycle operation is in progress for this sandbox",
            true,
        )
        .with_retry_after_ms(200)
    }

    #[must_use]
    pub fn agent_unavailable() -> Self {
        Self::new(
            502,
            "AGENT_UNAVAILABLE",
            "the sandbox runtime is temporarily unavailable",
            true,
        )
    }

    #[must_use]
    pub fn destroy_outcome_unknown(reason: &str) -> Self {
        Self::new(
            503,
            "SANDBOX_DESTROY_OUTCOME_UNKNOWN",
            "Sandbox destruction was not acknowledged by its host",
            true,
        )
        .with_details(serde_json::json!({"reason": reason}))
        .with_retry_after_ms(1_000)
    }

    /// A request the fast lane would have handed to the standard pipeline.
    ///
    /// The runner has no standard pipeline behind it, so it says so instead: the caller sends
    /// this request to `api.miosa.ai`, which still serves every shape.
    #[must_use]
    pub fn unsupported(what: &str) -> Self {
        Self::new(
            400,
            "RUNNER_UNSUPPORTED_REQUEST",
            &format!("the SOMA runner does not serve {what}; send this request to api.miosa.ai"),
            false,
        )
    }

    #[must_use]
    pub fn invalid_param(field: &str, message: &str) -> Self {
        Self::new(400, "INVALID_PARAM", message, false)
            .with_details(serde_json::json!({"field": field}))
    }
}

/// `Responder.unavailable/2`: the lane could not claim a prepared sandbox.
#[must_use]
pub fn create_unavailable() -> Vec<u8> {
    encode(&CapacityBody {
        error: CapacityFields {
            code: "SOMA_FAST_LANE_UNAVAILABLE",
            message: "The SOMA fast lane could not claim a prepared sandbox right now. Retry in a moment.",
            retry_after: 1,
            retryable: true,
        },
    })
}

/// A runner refusal from contract C2: `{"error":"<code>"}`.
#[must_use]
pub fn refusal(code: &str) -> Vec<u8> {
    encode(&serde_json::json!({ "error": code }))
}

/// The `421` body naming the runner that owns the sandbox.
#[must_use]
pub fn misdirected(host: &str) -> Vec<u8> {
    #[derive(Serialize)]
    struct Misdirected<'a> {
        error: &'a str,
        host: &'a str,
    }
    encode(&Misdirected {
        error: "misdirected",
        host,
    })
}

/// The `/healthz` body, in the field order contract C2 writes it.
#[must_use]
pub fn health(feed_age_ms: u64, pool_ready: Option<u64>) -> Vec<u8> {
    #[derive(Serialize)]
    struct Health {
        ok: bool,
        feed_age_ms: u64,
        pool_ready: Option<u64>,
    }
    encode(&Health {
        ok: true,
        feed_age_ms,
        pool_ready,
    })
}

#[must_use]
pub fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    // Every body here is built from strings, numbers, booleans, and nulls, which always encode.
    serde_json::to_vec(value).unwrap_or_else(|_| br#"{"error":"internal"}"#.to_vec())
}

#[cfg(test)]
#[path = "public_wire_tests.rs"]
mod tests;
