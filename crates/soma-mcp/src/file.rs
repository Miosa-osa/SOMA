//! The filesystem tool's input, request, and result.
//!
//! One tool serves all six operations rather than six tools. They take the same three inputs and
//! differ only in which of the remaining two they use, so six near-identical tool descriptions
//! would give an agent more to choose between without giving it more it can do.
//!
//! Content crosses as base64 in both directions, because guest bytes are not required to be
//! UTF-8 and a tool that carried them as text would corrupt every file that is not.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use soma::{FileAnswer, FileKind, FileMachineRequest, FileOperation, FileRefusal};

use crate::{BackendTarget, InstanceId, OperationId, input::BackendInput};

/// Largest path the guest protocol will carry, restated so the tool schema can bound its input.
const MAX_PATH_BYTES: usize = 4096;

/// Which of the six operations a call asks for.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FileOperationInput {
    Read,
    Write,
    Mkdir,
    List,
    Exists,
    Remove,
}

/// The filesystem tool's input.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileInput {
    #[schemars(length(equal = 32))]
    pub operation_id: Option<String>,
    #[schemars(length(equal = 32))]
    pub instance_id: String,
    pub operation: FileOperationInput,
    /// Absolute path inside the sandbox.
    #[schemars(length(min = 1, max = MAX_PATH_BYTES))]
    pub path: String,
    /// Base64 contents, for a write and for nothing else.
    #[serde(default)]
    pub content: Option<String>,
    /// Whether a remove may take a directory's contents with it.
    #[serde(default)]
    pub recursive: Option<bool>,
    #[serde(default)]
    #[schemars(default)]
    pub backend: BackendInput,
}

/// Why an input could not become a request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileInputError {
    Identity,
    MissingContent,
    UnexpectedContent,
    UnexpectedRecursive,
    Content,
    ContentTooLarge,
}

impl FileInputError {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Identity => "operation_id and instance_id must be 32 lowercase hex characters",
            Self::MissingContent => "a write must carry base64 content",
            Self::UnexpectedContent => "this operation does not take content",
            Self::UnexpectedRecursive => "this operation does not take a recursive flag",
            Self::Content => "content is not valid base64",
            Self::ContentTooLarge => "content exceeds the bytes one transfer will move",
        }
    }
}

/// One filesystem operation addressed to one managed sandbox.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileRequest {
    operation_id: OperationId,
    instance_id: InstanceId,
    operation: FileOperation,
    backend: BackendTarget,
}

impl FileRequest {
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

    /// Converts to the portable facade's own request.
    #[must_use]
    pub fn into_facade(self) -> FileMachineRequest {
        FileMachineRequest::new(self.operation_id, self.instance_id, self.operation)
    }
}

impl TryFrom<FileInput> for FileRequest {
    type Error = FileInputError;

    fn try_from(input: FileInput) -> Result<Self, Self::Error> {
        let path = input.path.into_bytes();
        let operation = match input.operation {
            FileOperationInput::Write => {
                refuse(
                    input.recursive.is_some(),
                    FileInputError::UnexpectedRecursive,
                )?;
                let encoded = input.content.ok_or(FileInputError::MissingContent)?;
                FileOperation::Write {
                    path,
                    bytes: decode(&encoded)?,
                }
            }
            FileOperationInput::Remove => {
                refuse(input.content.is_some(), FileInputError::UnexpectedContent)?;
                FileOperation::Remove {
                    path,
                    recursive: input.recursive.unwrap_or(false),
                }
            }
            other => {
                refuse(input.content.is_some(), FileInputError::UnexpectedContent)?;
                refuse(
                    input.recursive.is_some(),
                    FileInputError::UnexpectedRecursive,
                )?;
                match other {
                    FileOperationInput::Read => FileOperation::Read { path },
                    FileOperationInput::Mkdir => FileOperation::MakeDirectory { path },
                    FileOperationInput::List => FileOperation::ReadDirectory { path },
                    FileOperationInput::Exists => FileOperation::Exists { path },
                    FileOperationInput::Write | FileOperationInput::Remove => {
                        unreachable!("both are decided above")
                    }
                }
            }
        };
        Ok(Self {
            operation_id: input.operation_id.map_or_else(
                || Ok(crate::identity::generate_operation_id()),
                |value| OperationId::new(value).map_err(|_| FileInputError::Identity),
            )?,
            instance_id: InstanceId::new(input.instance_id)
                .map_err(|_| FileInputError::Identity)?,
            operation,
            backend: input.backend.into(),
        })
    }
}

const fn refuse(present: bool, error: FileInputError) -> Result<(), FileInputError> {
    if present { Err(error) } else { Ok(()) }
}

fn decode(encoded: &str) -> Result<Vec<u8>, FileInputError> {
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| FileInputError::Content)?;
    if bytes.len() > soma::MAX_FILE_BYTES {
        return Err(FileInputError::ContentTooLarge);
    }
    Ok(bytes)
}

/// One completed filesystem operation, as the tool reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileResult {
    instance_id: InstanceId,
    operation: &'static str,
    answer: FileAnswer,
}

impl FileResult {
    #[must_use]
    pub const fn new(instance_id: InstanceId, operation: &'static str, answer: FileAnswer) -> Self {
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

    /// The document this result serializes as.
    #[must_use]
    pub fn body(&self) -> FileBody<'_> {
        let mut body = FileBody {
            instance_id: self.instance_id.as_str(),
            operation: self.operation,
            refusal: None,
            content: None,
            byte_length: None,
            entries: None,
            more_entries: None,
            exists: None,
            kind: None,
        };
        match &self.answer {
            FileAnswer::Read { bytes } => body.content = Some(encoded(bytes)),
            FileAnswer::Written { bytes } => body.byte_length = Some(*bytes),
            FileAnswer::Listed { entries, more } => {
                body.entries = Some(
                    entries
                        .iter()
                        .map(|entry| EntryBody {
                            name: encoded(&entry.name),
                            kind: entry.kind.code(),
                        })
                        .collect(),
                );
                body.more_entries = Some(*more);
            }
            FileAnswer::Status { kind } => {
                body.exists = Some(kind.is_some());
                body.kind = kind.map(FileKind::code);
            }
            FileAnswer::Done => {}
            FileAnswer::Refused(refusal) => body.refusal = Some(FileRefusal::code(*refusal)),
        }
        body
    }

    /// Whether the guest declined the operation.
    #[must_use]
    pub const fn refused(&self) -> bool {
        matches!(self.answer, FileAnswer::Refused(_))
    }
}

/// Bytes on the wire, in the shape every other SOMA surface uses for guest bytes.
#[derive(Serialize)]
pub struct EncodedBytes {
    encoding: &'static str,
    byte_length: usize,
    data: String,
}

fn encoded(bytes: &[u8]) -> EncodedBytes {
    EncodedBytes {
        encoding: "base64",
        byte_length: bytes.len(),
        data: STANDARD.encode(bytes),
    }
}

#[derive(Serialize)]
pub struct EntryBody {
    name: EncodedBytes,
    kind: &'static str,
}

/// The filesystem tool's result document.
#[derive(Serialize)]
pub struct FileBody<'a> {
    instance_id: &'a str,
    operation: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    refusal: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<EncodedBytes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    byte_length: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    entries: Option<Vec<EntryBody>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    more_entries: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exists: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<&'static str>,
}
