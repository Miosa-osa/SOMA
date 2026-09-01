//! The terminal routes' request body and answer document.
//!
//! One body serves all five operations, for the reason the filesystem body serves six: they
//! differ only in which fields they need, and a field the named operation does not use is refused
//! rather than ignored.
//!
//! Bytes cross as base64 in both directions, exactly as command output and file contents already
//! do on this service. Terminal traffic is the least text-like thing here: it carries escape
//! sequences, control characters and whatever the program inside wrote, none of which is required
//! to be UTF-8.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use soma::{InstanceId, OperationId, PtyAnswer, PtyMachineRequest, PtyOperation, PtyRefusal};

use crate::{envelope::ApiError, report::OutputBytes, route::TerminalOperation};

/// The body every terminal route accepts.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalBody {
    /// The width in character cells, for an open and a resize.
    #[serde(default)]
    pub columns: Option<u16>,
    /// The height in character cells, for an open and a resize.
    #[serde(default)]
    pub rows: Option<u16>,
    /// The exact bytes a write types at the terminal, base64 encoded.
    #[serde(default)]
    pub input: Option<String>,
    /// Longest a read waits for the terminal's first byte, in milliseconds.
    #[serde(default)]
    pub wait_ms: Option<u32>,
    #[serde(default)]
    pub operation_id: Option<OperationId>,
}

impl TerminalBody {
    /// Converts the parsed body into the facade's terminal request.
    ///
    /// # Errors
    ///
    /// Returns a 400 refusal when a field the named operation needs is absent, when a field it
    /// does not use was supplied, when the input is not base64, or when the call is one the guest
    /// protocol will not carry.
    pub fn into_facade(
        self,
        instance_id: InstanceId,
        operation: TerminalOperation,
    ) -> Result<PtyMachineRequest, ApiError> {
        let operation = match operation {
            TerminalOperation::Open | TerminalOperation::Resize => {
                self.refuse_input()?;
                self.refuse_wait()?;
                let (columns, rows) = self.dimensions()?;
                if operation == TerminalOperation::Open {
                    PtyOperation::Open { columns, rows }
                } else {
                    PtyOperation::Resize { columns, rows }
                }
            }
            TerminalOperation::Write => {
                self.refuse_dimensions()?;
                self.refuse_wait()?;
                let encoded = self
                    .input
                    .as_deref()
                    .ok_or_else(|| ApiError::invalid("a write must carry the input to type"))?;
                PtyOperation::Write {
                    bytes: decode(encoded)?,
                }
            }
            TerminalOperation::Read => {
                self.refuse_dimensions()?;
                self.refuse_input()?;
                PtyOperation::Read {
                    wait_millis: self.wait_ms.unwrap_or(0),
                }
            }
            TerminalOperation::Close => {
                self.refuse_dimensions()?;
                self.refuse_input()?;
                self.refuse_wait()?;
                PtyOperation::Close
            }
        };
        operation
            .check()
            .map_err(|rejected| ApiError::invalid(rejected.message()))?;
        Ok(PtyMachineRequest::new(
            crate::wire::operation_id(self.operation_id)?,
            instance_id,
            operation,
        ))
    }

    fn dimensions(&self) -> Result<(u16, u16), ApiError> {
        match (self.columns, self.rows) {
            (Some(columns), Some(rows)) => Ok((columns, rows)),
            _ => Err(ApiError::invalid(
                "this operation needs both columns and rows",
            )),
        }
    }

    fn refuse_dimensions(&self) -> Result<(), ApiError> {
        if self.columns.is_some() || self.rows.is_some() {
            return Err(ApiError::invalid("this operation does not take dimensions"));
        }
        Ok(())
    }

    fn refuse_input(&self) -> Result<(), ApiError> {
        if self.input.is_some() {
            return Err(ApiError::invalid("this operation does not take input"));
        }
        Ok(())
    }

    fn refuse_wait(&self) -> Result<(), ApiError> {
        if self.wait_ms.is_some() {
            return Err(ApiError::invalid("this operation does not take a wait"));
        }
        Ok(())
    }
}

fn decode(encoded: &str) -> Result<Vec<u8>, ApiError> {
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| ApiError::invalid("the input field is not valid base64"))?;
    if bytes.len() > soma::MAX_PTY_CHUNK_BYTES {
        return Err(ApiError::new(
            413,
            "input_too_large",
            "the input exceeds the bytes one terminal call carries",
            false,
        ));
    }
    Ok(bytes)
}

/// What one terminal operation answered.
///
/// The document is one shape with optional members rather than five, so a client reads `refusal`
/// to learn whether the operation happened without having to know which one it asked for. Every
/// member absent from an answer is absent from the document.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TerminalReport {
    pub instance_id: InstanceId,
    pub operation: &'static str,
    /// The typed cause when the guest declined, and absent when it did not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
    /// How many leading bytes of a write the terminal accepted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub written: Option<u32>,
    /// One bounded chunk of terminal output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<OutputBytes>,
    /// Whether the session has ended and no further byte will follow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<bool>,
}

impl TerminalReport {
    /// Builds the document for one answer.
    #[must_use]
    pub fn new(instance_id: InstanceId, operation: &'static str, answer: &PtyAnswer) -> Self {
        let mut report = Self {
            instance_id,
            operation,
            refusal: None,
            columns: None,
            rows: None,
            written: None,
            output: None,
            ended: None,
        };
        match answer {
            PtyAnswer::Opened { columns, rows } | PtyAnswer::Resized { columns, rows } => {
                report.columns = Some(*columns);
                report.rows = Some(*rows);
            }
            PtyAnswer::Wrote { bytes } => report.written = Some(*bytes),
            PtyAnswer::Output { bytes, end } => {
                report.output = Some(OutputBytes::new(bytes));
                report.ended = Some(*end);
            }
            PtyAnswer::Closed => {}
            PtyAnswer::Refused(refusal) => report.refusal = Some(PtyRefusal::code(*refusal)),
        }
        report
    }
}
