//! Response construction and request-body plumbing shared by every handler.
//!
//! These are the pieces every route needs and none of them owns: how a success is rendered,
//! how a refusal is, and how a JSON body is parsed under the facade's reject-unknown-fields
//! rule. They live apart so the handler file stays inside the source budget.

use serde::{Serialize, de::DeserializeOwned};
use soma::ExecutionReceipt;

use crate::envelope::{ApiError, Envelope, failure_body, render};
use crate::http::response::Response;
use crate::wire::ControlBody;

pub(super) fn success<T: Serialize>(
    status: u16,
    operation: &'static str,
    result: &T,
    receipt: Option<&ExecutionReceipt>,
) -> Response {
    Response::new(
        status,
        render(&Envelope::success(operation, result, receipt)),
    )
}

fn refuse(operation: &'static str, error: ApiError) -> Response {
    Response::new(error.status(), failure_body(operation, error))
}

/// Parses a required JSON body.
///
/// The facade's wire types reject unknown fields, so a caller that misspells a field is told so
/// rather than having its intent silently dropped.
fn parse<T: DeserializeOwned>(body: &[u8]) -> Result<T, ApiError> {
    serde_json::from_slice(body)
        .map_err(|_| ApiError::invalid("the request body is not a valid document for this route"))
}

/// Parses an optional control body, treating an absent body as an empty one.
///
/// Control routes need nothing from the caller beyond the path, so requiring `{}` would be
/// ceremony; supplying an operation id stays available for a caller that wants to choose it.
fn control_body(body: &[u8]) -> Result<ControlBody, ApiError> {
    optional_body(body)
}

/// Parses a body whose every field has a default, treating an absent body as an empty one.
///
/// A terminal close and a terminal read with no wait need nothing from the caller beyond the
/// path, so requiring `{}` would be ceremony.
fn optional_body<T: DeserializeOwned + Default>(body: &[u8]) -> Result<T, ApiError> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(T::default());
    }
    parse(body)
}
