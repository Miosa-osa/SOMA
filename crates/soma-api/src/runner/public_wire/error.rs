//! The platform API error body, and the errors the runner answers with.
//!
//! It lives apart from the success bodies so the wire module stays inside the source budget; the
//! bytes are unchanged, and `public_wire::PlatformError` is still the one name callers use.

use serde::Serialize;
use serde_json::Value;

use super::encode;

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

    /// A create field the platform does not define. The answer names the field, so a caller
    /// that misspelled one is told which, instead of a create that quietly ignored it.
    #[must_use]
    pub fn unknown_field(field: &str) -> Self {
        Self::invalid_param(
            field,
            &format!("unknown parameter: the create body does not define `{field}`"),
        )
    }
}
