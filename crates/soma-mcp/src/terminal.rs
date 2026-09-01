//! The bounded terminal tool's input, request, and result.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use soma::{PtyAnswer, PtyMachineRequest, PtyOperation, PtyRefusal};

use crate::{BackendTarget, InstanceId, OperationId, input::BackendInput};

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TerminalOperationInput {
    Open,
    Write,
    Read,
    Resize,
    Close,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TerminalInput {
    #[schemars(length(equal = 32))]
    pub operation_id: Option<String>,
    #[schemars(length(equal = 32))]
    pub instance_id: String,
    pub operation: TerminalOperationInput,
    pub columns: Option<u16>,
    pub rows: Option<u16>,
    /// Base64 terminal input, used only by `write`.
    pub input: Option<String>,
    pub wait_ms: Option<u32>,
    #[serde(default)]
    #[schemars(default)]
    pub backend: BackendInput,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalInputError {
    Identity,
    MissingDimensions,
    UnexpectedDimensions,
    MissingInput,
    UnexpectedInput,
    UnexpectedWait,
    Input,
    Operation,
}

impl TerminalInputError {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Identity => "operation_id and instance_id must be 32 lowercase hex characters",
            Self::MissingDimensions => "open and resize require both columns and rows",
            Self::UnexpectedDimensions => "this terminal operation does not take dimensions",
            Self::MissingInput => "a terminal write requires base64 input",
            Self::UnexpectedInput => "this terminal operation does not take input",
            Self::UnexpectedWait => "this terminal operation does not take a wait",
            Self::Input => "terminal input is not valid base64",
            Self::Operation => "the terminal operation exceeds the guest protocol bounds",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalRequest {
    operation_id: OperationId,
    instance_id: InstanceId,
    operation: PtyOperation,
    backend: BackendTarget,
}

impl TerminalRequest {
    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    #[must_use]
    pub const fn backend(&self) -> BackendTarget {
        self.backend
    }

    #[must_use]
    pub fn into_facade(self) -> PtyMachineRequest {
        PtyMachineRequest::new(self.operation_id, self.instance_id, self.operation)
    }
}

impl TryFrom<TerminalInput> for TerminalRequest {
    type Error = TerminalInputError;

    fn try_from(input: TerminalInput) -> Result<Self, Self::Error> {
        let operation = operation(&input)?;
        operation
            .check()
            .map_err(|_| TerminalInputError::Operation)?;
        Ok(Self {
            operation_id: input.operation_id.map_or_else(
                || Ok(crate::identity::generate_operation_id()),
                |value| OperationId::new(value).map_err(|_| TerminalInputError::Identity),
            )?,
            instance_id: InstanceId::new(input.instance_id)
                .map_err(|_| TerminalInputError::Identity)?,
            operation,
            backend: input.backend.into(),
        })
    }
}

fn operation(input: &TerminalInput) -> Result<PtyOperation, TerminalInputError> {
    match input.operation {
        TerminalOperationInput::Open | TerminalOperationInput::Resize => {
            refuse(input.input.is_some(), TerminalInputError::UnexpectedInput)?;
            refuse(input.wait_ms.is_some(), TerminalInputError::UnexpectedWait)?;
            let (Some(columns), Some(rows)) = (input.columns, input.rows) else {
                return Err(TerminalInputError::MissingDimensions);
            };
            Ok(if input.operation == TerminalOperationInput::Open {
                PtyOperation::Open { columns, rows }
            } else {
                PtyOperation::Resize { columns, rows }
            })
        }
        TerminalOperationInput::Write => {
            refuse_dimensions(input)?;
            refuse(input.wait_ms.is_some(), TerminalInputError::UnexpectedWait)?;
            let bytes = STANDARD
                .decode(
                    input
                        .input
                        .as_deref()
                        .ok_or(TerminalInputError::MissingInput)?,
                )
                .map_err(|_| TerminalInputError::Input)?;
            Ok(PtyOperation::Write { bytes })
        }
        TerminalOperationInput::Read => {
            refuse_dimensions(input)?;
            refuse(input.input.is_some(), TerminalInputError::UnexpectedInput)?;
            Ok(PtyOperation::Read {
                wait_millis: input.wait_ms.unwrap_or(0),
            })
        }
        TerminalOperationInput::Close => {
            refuse_dimensions(input)?;
            refuse(input.input.is_some(), TerminalInputError::UnexpectedInput)?;
            refuse(input.wait_ms.is_some(), TerminalInputError::UnexpectedWait)?;
            Ok(PtyOperation::Close)
        }
    }
}

fn refuse_dimensions(input: &TerminalInput) -> Result<(), TerminalInputError> {
    refuse(
        input.columns.is_some() || input.rows.is_some(),
        TerminalInputError::UnexpectedDimensions,
    )
}

const fn refuse(present: bool, error: TerminalInputError) -> Result<(), TerminalInputError> {
    if present { Err(error) } else { Ok(()) }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalResult {
    instance_id: InstanceId,
    operation: &'static str,
    answer: PtyAnswer,
}

impl TerminalResult {
    #[must_use]
    pub const fn new(instance_id: InstanceId, operation: &'static str, answer: PtyAnswer) -> Self {
        Self {
            instance_id,
            operation,
            answer,
        }
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    #[must_use]
    pub const fn refused(&self) -> bool {
        matches!(self.answer, PtyAnswer::Refused(_))
    }

    #[must_use]
    pub fn body(&self) -> TerminalBody<'_> {
        let mut body = TerminalBody {
            instance_id: self.instance_id.as_str(),
            operation: self.operation,
            refusal: None,
            columns: None,
            rows: None,
            written: None,
            output: None,
            ended: None,
        };
        match &self.answer {
            PtyAnswer::Opened { columns, rows } | PtyAnswer::Resized { columns, rows } => {
                body.columns = Some(*columns);
                body.rows = Some(*rows);
            }
            PtyAnswer::Wrote { bytes } => body.written = Some(*bytes),
            PtyAnswer::Output { bytes, end } => {
                body.output = Some(EncodedBytes::new(bytes));
                body.ended = Some(*end);
            }
            PtyAnswer::Closed => {}
            PtyAnswer::Refused(refusal) => body.refusal = Some(PtyRefusal::code(*refusal)),
        }
        body
    }
}

#[derive(Serialize)]
pub struct EncodedBytes {
    encoding: &'static str,
    byte_length: usize,
    data: String,
}

impl EncodedBytes {
    fn new(bytes: &[u8]) -> Self {
        Self {
            encoding: "base64",
            byte_length: bytes.len(),
            data: STANDARD.encode(bytes),
        }
    }
}

#[derive(Serialize)]
pub struct TerminalBody<'a> {
    instance_id: &'a str,
    operation: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    refusal: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    columns: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rows: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    written: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<EncodedBytes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ended: Option<bool>,
}
