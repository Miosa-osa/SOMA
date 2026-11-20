//! The bodies a file call answers with, byte-compatible with the platform's file actions.
//!
//! The platform renders these as small Elixir maps through Jason, which writes a small map's keys
//! in sorted order, so every struct here declares its fields alphabetically and `serde_json`
//! writes the same bytes, as the rest of the runner's public bodies do.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;
use soma::{FileEntry, FileKind};

use crate::runner::clock::iso8601_seconds;

/// One listing entry: name, kind, and the size and time when the guest reported them.
#[derive(Serialize)]
pub(crate) struct ListedEntry {
    /// `"unavailable"` when the guest reported no size or time, which a client must read as
    /// unknown rather than as zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    modified_at: Option<String>,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_bytes: Option<u64>,
    #[serde(rename = "type")]
    kind: &'static str,
}

impl ListedEntry {
    /// The entry, or `None` for a name JSON text cannot carry exactly.
    ///
    /// A guest name is bytes. One that is not UTF-8 cannot be written into this body without
    /// changing it, and a changed name names a file that does not exist, so it is left out.
    pub(crate) fn of(entry: &FileEntry) -> Option<Self> {
        let name = String::from_utf8(entry.name.clone()).ok()?;
        let kind = kind_name(entry.kind);
        Some(match entry.metadata {
            Some(metadata) => Self {
                metadata: None,
                modified_at: Some(iso8601_seconds(metadata.modified)),
                name,
                size_bytes: Some(metadata.size),
                kind,
            },
            None => Self {
                metadata: Some("unavailable"),
                modified_at: None,
                name,
                size_bytes: None,
                kind,
            },
        })
    }
}

/// `GET fs`: `%{files: entries, path: path}`.
#[derive(Serialize)]
pub(crate) struct Listing<'a> {
    pub(crate) files: Vec<ListedEntry>,
    pub(crate) path: &'a str,
    /// Present, and true, only when the directory held more entries than one listing carries.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) truncated: bool,
}

/// `GET fs/read`: the contents as text when they are UTF-8, and as base64 when they are not.
#[derive(Serialize)]
pub(crate) struct Contents<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_base64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    encoding: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    length: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    offset: Option<u64>,
    path: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
}

impl<'a> Contents<'a> {
    pub(crate) fn whole(bytes: Vec<u8>, path: &'a str) -> Self {
        let (content, content_base64, encoding) = match String::from_utf8(bytes) {
            Ok(text) => (Some(text), None, None),
            Err(error) => (
                None,
                Some(STANDARD.encode(error.into_bytes())),
                Some("base64"),
            ),
        };
        Self {
            content,
            content_base64,
            encoding,
            length: None,
            offset: None,
            path,
            size: None,
        }
    }

    pub(crate) fn part(bytes: Vec<u8>, path: &'a str, offset: u64, size: u64) -> Self {
        let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        Self {
            length: Some(length),
            offset: Some(offset),
            size: Some(size),
            ..Self::whole(bytes, path)
        }
    }
}

/// `POST fs/write`: `%{file: %{filename, path, size_bytes}, success: true}`.
#[derive(Serialize)]
pub(crate) struct WrittenFile<'a> {
    pub(crate) file: WrittenFields<'a>,
    pub(crate) success: bool,
}

#[derive(Serialize)]
pub(crate) struct WrittenFields<'a> {
    pub(crate) filename: &'a str,
    pub(crate) path: &'a str,
    pub(crate) size_bytes: u64,
}

/// `POST fs/stat`: the fields the platform's stat script prints, under `data`.
#[derive(Serialize)]
pub(crate) struct StatData<'a> {
    pub(crate) is_dir: bool,
    pub(crate) is_file: bool,
    /// The guest reports a symbolic link as neither a file nor a directory and does not say
    /// which other kind it is, so this is false, as on the platform's other SOMA road.
    pub(crate) is_symlink: bool,
    pub(crate) mode: String,
    pub(crate) modified_at: String,
    pub(crate) name: &'a str,
    pub(crate) path: &'a str,
    pub(crate) size: u64,
    pub(crate) size_bytes: u64,
}

/// `%{data: data}`, the envelope the platform's stat, mkdir, rename, and chmod answers share.
#[derive(Serialize)]
pub(crate) struct Data<T> {
    pub(crate) data: T,
}

/// What `mkdir` answers inside `data`: the guest daemon's own `{"status": "ok"}`.
#[derive(Serialize)]
pub(crate) struct Status {
    pub(crate) status: &'static str,
}

/// What `rename` answers inside `data`.
#[derive(Serialize)]
pub(crate) struct Moved<'a> {
    pub(crate) from: &'a str,
    pub(crate) to: &'a str,
}

/// What `chmod` answers inside `data`.
#[derive(Serialize)]
pub(crate) struct ModeSet<'a> {
    pub(crate) mode: &'a str,
    pub(crate) path: &'a str,
}

/// `DELETE fs`: `%{path: path, success: true}`.
#[derive(Serialize)]
pub(crate) struct Deleted<'a> {
    pub(crate) path: &'a str,
    pub(crate) success: bool,
}

/// The platform's names for a kind: a directory, and everything else as a file.
const fn kind_name(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Directory => "directory",
        FileKind::File | FileKind::Other => "file",
    }
}
