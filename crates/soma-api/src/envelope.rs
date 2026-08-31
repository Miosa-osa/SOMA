use serde::Serialize;

/// Wire contract name for this service, kept parallel to the CLI's `soma.cli.v1`.
pub const ENVELOPE_SCHEMA: &str = "soma.api.v1";

/// A failure body with exactly the fields the CLI already publishes, so an operator reading a
/// CLI JSON failure and an API failure reads the same three fields with the same meanings.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct FailureBody {
    pub code: &'static str,
    pub message: &'static str,
    pub retryable: bool,
}

/// A refusal or failure paired with the HTTP status that carries it.
///
/// The status lives beside the body rather than being derived from the code at render time,
/// because the same code can be correct under more than one status only if a caller is told
/// which one, and one struct removes the chance of the two drifting apart.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApiError {
    status: u16,
    body: FailureBody,
}

impl ApiError {
    #[must_use]
    pub const fn new(
        status: u16,
        code: &'static str,
        message: &'static str,
        retryable: bool,
    ) -> Self {
        Self {
            status,
            body: FailureBody {
                code,
                message,
                retryable,
            },
        }
    }

    #[must_use]
    pub const fn status(self) -> u16 {
        self.status
    }

    #[must_use]
    pub const fn body(self) -> FailureBody {
        self.body
    }

    #[must_use]
    pub const fn not_found() -> Self {
        Self::new(404, "route_not_found", "no such API route", false)
    }

    #[must_use]
    pub const fn method_not_allowed() -> Self {
        Self::new(
            405,
            "method_not_allowed",
            "the route does not accept this method",
            false,
        )
    }

    #[must_use]
    pub const fn invalid(message: &'static str) -> Self {
        Self::new(400, "invalid_input", message, false)
    }

    #[must_use]
    pub const fn internal(message: &'static str) -> Self {
        Self::new(500, "internal_contract_failure", message, false)
    }
}

/// The single response shape of this service.
///
/// Success and failure share one envelope, as they do in the CLI, so a client parses one schema
/// and reads `status` rather than branching on which of two documents it received.
#[derive(Serialize)]
pub struct Envelope<'a, T: Serialize> {
    schema: &'static str,
    operation: &'static str,
    status: &'static str,
    result: Option<&'a T>,
    error: Option<FailureBody>,
    receipt: Option<&'a soma::ExecutionReceipt>,
}

impl<'a, T: Serialize> Envelope<'a, T> {
    #[must_use]
    pub const fn success(
        operation: &'static str,
        result: &'a T,
        receipt: Option<&'a soma::ExecutionReceipt>,
    ) -> Self {
        Self {
            schema: ENVELOPE_SCHEMA,
            operation,
            status: "ok",
            result: Some(result),
            error: None,
            receipt,
        }
    }
}

/// Renders a failure envelope without a result payload.
///
/// Failures are rendered through a concrete unit type so callers never have to name a result
/// type they do not have when the request failed before producing one.
#[must_use]
pub fn failure_body(operation: &'static str, error: ApiError) -> Vec<u8> {
    let envelope: Envelope<'_, ()> = Envelope {
        schema: ENVELOPE_SCHEMA,
        operation,
        status: "error",
        result: None,
        error: Some(error.body()),
        receipt: None,
    };
    render(&envelope)
}

/// Serializes an envelope, degrading to a fixed internal failure document if serialization fails.
///
/// A response body must always exist, so a serialization fault becomes a valid failure envelope
/// rather than a dropped connection that tells the caller nothing.
#[must_use]
pub fn render<T: Serialize>(envelope: &Envelope<'_, T>) -> Vec<u8> {
    serde_json::to_vec(envelope).unwrap_or_else(|_| {
        br#"{"schema":"soma.api.v1","operation":"unknown","status":"error","result":null,"error":{"code":"internal_contract_failure","message":"the response could not be encoded","retryable":false},"receipt":null}"#
            .to_vec()
    })
}
