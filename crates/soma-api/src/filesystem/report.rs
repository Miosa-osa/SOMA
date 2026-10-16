//! The answer document one filesystem route sends, and the report each answer becomes.
//!
//! It lives apart from the request body so the body and the document each stay inside the source
//! budget; a child module may read the parent's imports, so the split costs nothing but a `use`.

use serde::Serialize;
use soma::{FileAnswer, FileEntry, FileKind, FileRefusal};

use crate::report::OutputBytes;

use super::*;

/// One directory entry on the wire.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EntryReport {
    pub name: OutputBytes,
    pub kind: &'static str,
    /// Size in bytes, absent when the guest did not offer it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// Permission bits, absent when the guest did not offer them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    /// Modification time in seconds since the Unix epoch, absent when the guest did not offer it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<u64>,
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
    /// `"unavailable"` when the guest could not report size, mode, and time for the listing.
    ///
    /// A client must read an entry without `size` as unknown, never as zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<&'static str>,
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
            metadata: None,
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
                if entries.iter().any(|entry| entry.metadata.is_none()) {
                    report.metadata = Some("unavailable");
                }
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
        size: entry.metadata.map(|metadata| metadata.size),
        mode: entry.metadata.map(|metadata| metadata.mode),
        modified: entry.metadata.map(|metadata| metadata.modified),
    }
}
