//! What the envelope says about one filesystem operation.
//!
//! It sits apart from the lifecycle reports because it is the only one whose answer varies by
//! operation: a read has bytes, a listing has entries, and an existence check has neither, so the
//! document is built from the answer rather than declared once for every call.

use serde::Serialize;
use soma::{FileAnswer, FileKind, FileRefusal, InstanceId};

use super::OutputBytes;

/// One directory entry as the envelope reports it.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct EntryReport {
    pub name: OutputBytes,
    pub kind: &'static str,
}

/// One completed filesystem operation.
///
/// Every member a given operation does not answer with is absent from the document rather than
/// present and empty, so a reader cannot mistake "this operation says nothing about it" for
/// "this operation says it is empty".
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct FileReport {
    pub instance_id: InstanceId,
    pub operation: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<OutputBytes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub byte_length: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<EntryReport>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub more_entries: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exists: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<&'static str>,
}

impl FileReport {
    /// Builds the report for one answer.
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
                report.entries = Some(
                    entries
                        .iter()
                        .map(|entry| EntryReport {
                            name: OutputBytes::new(&entry.name),
                            kind: entry.kind.code(),
                        })
                        .collect(),
                );
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

    /// Whether the guest declined the operation.
    #[must_use]
    pub const fn refused(&self) -> bool {
        self.refusal.is_some()
    }
}
