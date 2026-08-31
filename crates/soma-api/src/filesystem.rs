//! The filesystem routes' request body and answer document.
//!
//! One body serves all six operations, because they differ only in which fields they need and a
//! separate document per operation would repeat the instance, the path, and the operation
//! identity six times. A field the named operation does not use is refused rather than ignored,
//! so a caller that sent `content` to a directory listing learns it did.
//!
//! Contents cross as base64, exactly as command output already does on this service, because
//! guest bytes are not required to be UTF-8 and a surface that carried them as text would corrupt
//! every file that is not. Paths cross as JSON strings: a guest path is bytes and the guest still
//! validates it as bytes, but a JSON document cannot carry a byte string that is not UTF-8, so a
//! path this service can express is a UTF-8 one. The CLI passes the operating system's own bytes
//! and is not narrowed this way.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use soma::{
    FileAnswer, FileEntry, FileKind, FileMachineRequest, FileOperation, FileRefusal, InstanceId,
    OperationId,
};

use crate::{envelope::ApiError, report::OutputBytes, route::FilesystemOperation};

/// The body every filesystem route accepts.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesystemBody {
    pub path: String,
    /// The exact bytes a write puts in the file, base64 encoded.
    #[serde(default)]
    pub content: Option<String>,
    /// Whether a remove may take a directory's contents with it.
    #[serde(default)]
    pub recursive: Option<bool>,
    #[serde(default)]
    pub operation_id: Option<OperationId>,
}

impl FilesystemBody {
    /// Converts the parsed body into the facade's filesystem request.
    ///
    /// # Errors
    ///
    /// Returns a 400 refusal when the path is one the guest protocol will not carry, when a field
    /// the named operation does not use was supplied, or when a write carried no content or
    /// content that is not base64, and a 413 when the content exceeds the bytes one transfer
    /// will move.
    pub fn into_facade(
        self,
        instance_id: InstanceId,
        operation: FilesystemOperation,
    ) -> Result<FileMachineRequest, ApiError> {
        let path = self.path.into_bytes();
        soma::check_guest_path(&path).map_err(|rejected| ApiError::invalid(rejected.message()))?;
        let operation = match operation {
            FilesystemOperation::Write => {
                let encoded = self
                    .content
                    .ok_or_else(|| ApiError::invalid("a write must carry the content to write"))?;
                refuse_recursive(self.recursive.is_some())?;
                FileOperation::Write {
                    path,
                    bytes: decode(&encoded)?,
                }
            }
            FilesystemOperation::Remove => {
                refuse_content(self.content.is_some())?;
                FileOperation::Remove {
                    path,
                    recursive: self.recursive.unwrap_or(false),
                }
            }
            other => {
                refuse_content(self.content.is_some())?;
                refuse_recursive(self.recursive.is_some())?;
                match other {
                    FilesystemOperation::Read => FileOperation::Read { path },
                    FilesystemOperation::List => FileOperation::ReadDirectory { path },
                    FilesystemOperation::MakeDirectory => FileOperation::MakeDirectory { path },
                    FilesystemOperation::Exists => FileOperation::Exists { path },
                    FilesystemOperation::Write | FilesystemOperation::Remove => {
                        unreachable!("both are decided above")
                    }
                }
            }
        };
        Ok(FileMachineRequest::new(
            crate::wire::operation_id(self.operation_id)?,
            instance_id,
            operation,
        ))
    }
}

fn refuse_content(present: bool) -> Result<(), ApiError> {
    if present {
        return Err(ApiError::invalid("this operation does not take content"));
    }
    Ok(())
}

fn refuse_recursive(present: bool) -> Result<(), ApiError> {
    if present {
        return Err(ApiError::invalid(
            "this operation does not take a recursive flag",
        ));
    }
    Ok(())
}

fn decode(encoded: &str) -> Result<Vec<u8>, ApiError> {
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| ApiError::invalid("the content field is not valid base64"))?;
    if bytes.len() > soma::MAX_FILE_BYTES {
        return Err(ApiError::new(
            413,
            "content_too_large",
            "the content exceeds the bytes one transfer will move",
            false,
        ));
    }
    Ok(bytes)
}

/// One directory entry on the wire.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EntryReport {
    pub name: OutputBytes,
    pub kind: &'static str,
}

/// What one filesystem operation answered.
///
/// The document is one shape with optional members rather than six shapes, so a client reads
/// `refusal` to learn whether the operation happened without having to know which operation it
/// asked for. Every member absent from an answer is absent from the document.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FilesystemReport {
    pub instance_id: InstanceId,
    pub operation: &'static str,
    /// The typed cause when the guest declined, and absent when it did not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<OutputBytes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub byte_length: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<EntryReport>>,
    /// Whether the directory held more entries than this listing carries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub more_entries: Option<bool>,
    /// Whether the path exists, for the operation that asks exactly that.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exists: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<&'static str>,
}

impl FilesystemReport {
    /// Builds the document for one answer.
    #[must_use]
    pub fn new(instance_id: InstanceId, operation: &'static str, answer: &FileAnswer) -> Self {
        let mut report = Self {
            instance_id,
            operation,
            refusal: None,
            content: None,
            byte_length: None,
            entries: None,
            more_entries: None,
            exists: None,
            kind: None,
        };
        match answer {
            FileAnswer::Read { bytes } => report.content = Some(OutputBytes::new(bytes)),
            FileAnswer::Written { bytes } => report.byte_length = Some(*bytes),
            FileAnswer::Listed { entries, more } => {
                report.entries = Some(entries.iter().map(entry).collect());
                report.more_entries = Some(*more);
            }
            FileAnswer::Status { kind } => {
                report.exists = Some(kind.is_some());
                report.kind = kind.map(FileKind::code);
            }
            FileAnswer::Done => {}
            FileAnswer::Refused(refusal) => report.refusal = Some(FileRefusal::code(*refusal)),
        }
        report
    }
}

fn entry(entry: &FileEntry) -> EntryReport {
    EntryReport {
        name: OutputBytes::new(&entry.name),
        kind: entry.kind.code(),
    }
}
