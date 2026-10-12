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
    /// The destination of a rename.
    #[serde(default)]
    pub to: Option<String>,
    /// Where a ranged read or write starts.
    #[serde(default)]
    pub offset: Option<u64>,
    /// How many bytes a ranged read asks for.
    #[serde(default)]
    pub length: Option<u32>,
    /// The permission bits a chmod applies.
    #[serde(default)]
    pub mode: Option<u32>,
    /// Whether a ranged write creates a missing file.
    #[serde(default)]
    pub create: Option<bool>,
    /// Whether a ranged write ends the file where it ends.
    #[serde(default)]
    pub shorten: Option<bool>,
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
        self.refuse_unused_fields(operation)?;
        let path = self.path.clone().into_bytes();
        soma::check_guest_path(&path).map_err(|rejected| ApiError::invalid(rejected.message()))?;
        let operation = match operation {
            FilesystemOperation::Stat
            | FilesystemOperation::Rename
            | FilesystemOperation::Chmod
            | FilesystemOperation::ReadRange
            | FilesystemOperation::WriteRange => self.level_two(operation, path)?,
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
                    _ => unreachable!("all are decided above"),
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

impl FilesystemBody {
    /// Builds a guest API level 2 operation: stat, rename, chmod, and the ranged transfers.
    fn level_two(
        &self,
        operation: FilesystemOperation,
        path: Vec<u8>,
    ) -> Result<FileOperation, ApiError> {
        refuse_recursive(self.recursive.is_some())?;
        if operation != FilesystemOperation::WriteRange {
            refuse_content(self.content.is_some())?;
        }
        Ok(match operation {
            FilesystemOperation::Rename => {
                let to = self
                    .to
                    .clone()
                    .ok_or_else(|| ApiError::invalid("a rename must name its destination"))?
                    .into_bytes();
                soma::check_guest_path(&to)
                    .map_err(|rejected| ApiError::invalid(rejected.message()))?;
                FileOperation::Rename { path, to }
            }
            FilesystemOperation::Chmod => {
                let mode = self
                    .mode
                    .ok_or_else(|| ApiError::invalid("a chmod must carry the mode"))?;
                if mode > 0o777 {
                    return Err(ApiError::invalid("the mode must be within 0o777"));
                }
                FileOperation::SetMode { path, mode }
            }
            FilesystemOperation::ReadRange => {
                let length = self
                    .length
                    .ok_or_else(|| ApiError::invalid("a ranged read must carry a length"))?;
                if usize::try_from(length).map_or(true, |length| length > soma::MAX_FILE_BYTES) {
                    return Err(ApiError::invalid("the length exceeds one transfer"));
                }
                FileOperation::ReadRange {
                    path,
                    offset: self.offset.unwrap_or(0),
                    length,
                }
            }
            FilesystemOperation::WriteRange => {
                let encoded = self.content.as_deref().ok_or_else(|| {
                    ApiError::invalid("a ranged write must carry the content to write")
                })?;
                FileOperation::WriteRange {
                    path,
                    offset: self.offset.unwrap_or(0),
                    create: self.create.unwrap_or(false),
                    shorten: self.shorten.unwrap_or(false),
                    bytes: decode(encoded)?,
                }
            }
            _ => FileOperation::Stat { path },
        })
    }

    /// Refuses a field the named operation does not use, so a mistyped call is told so.
    fn refuse_unused_fields(&self, operation: FilesystemOperation) -> Result<(), ApiError> {
        use FilesystemOperation as Op;
        let allowed = |field: &str| match field {
            "to" => operation == Op::Rename,
            "offset" => matches!(operation, Op::ReadRange | Op::WriteRange),
            "length" => operation == Op::ReadRange,
            "mode" => operation == Op::Chmod,
            "create" | "shorten" => operation == Op::WriteRange,
            _ => false,
        };
        let present = [
            ("to", self.to.is_some()),
            ("offset", self.offset.is_some()),
            ("length", self.length.is_some()),
            ("mode", self.mode.is_some()),
            ("create", self.create.is_some()),
            ("shorten", self.shorten.is_some()),
        ];
        if present.iter().any(|(field, set)| *set && !allowed(field)) {
            return Err(ApiError::invalid(
                "this operation does not take one of the fields supplied",
            ));
        }
        Ok(())
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
    /// Size in bytes, for a stat.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// Permission bits, for a stat.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    /// Modification time in seconds since the Unix epoch, for a stat.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<u64>,
    /// Whether a ranged read reached the end of the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_of_file: Option<bool>,
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
            size: None,
            mode: None,
            modified: None,
            end_of_file: None,
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
            FileAnswer::Range { bytes, end } => {
                report.content = Some(OutputBytes::new(bytes));
                report.end_of_file = Some(*end);
            }
            FileAnswer::Stat {
                kind,
                size,
                mode,
                modified,
            } => {
                report.kind = Some(kind.code());
                report.size = Some(*size);
                report.mode = Some(*mode);
                report.modified = Some(*modified);
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
