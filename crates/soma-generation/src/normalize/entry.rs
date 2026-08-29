use soma::OciDigest;

use crate::{NormalizeError, NormalizeErrorKind, NormalizePhase};

pub(super) type GuestPath = Vec<u8>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Metadata {
    pub(super) mode: u32,
    pub(super) uid: u32,
    pub(super) gid: u32,
    pub(super) mtime: u64,
}

impl Metadata {
    pub(super) const fn implicit_directory() -> Self {
        Self {
            mode: 0o755,
            uid: 0,
            gid: 0,
            mtime: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum PlannedNode {
    Directory(Metadata),
    Regular {
        metadata: Metadata,
        digest: OciDigest,
        size: u64,
    },
    Symlink {
        metadata: Metadata,
        target: Vec<u8>,
    },
    Hardlink {
        target: GuestPath,
    },
    Fifo(Metadata),
}

pub(super) fn normalize_path(path: &[u8], maximum: u32) -> Result<GuestPath, NormalizeError> {
    validate_bytes(path, maximum)?;
    if path.is_empty() || path.starts_with(b"/") {
        return Err(invalid());
    }
    let mut normalized = Vec::with_capacity(path.len());
    for component in path.split(|byte| *byte == b'/') {
        match component {
            b"" | b"." => {}
            b".." => return Err(invalid()),
            value => {
                if !normalized.is_empty() {
                    normalized.push(b'/');
                }
                normalized.extend_from_slice(value);
            }
        }
    }
    Ok(normalized)
}

pub(super) fn validate_link(value: &[u8], maximum: u32) -> Result<Vec<u8>, NormalizeError> {
    validate_bytes(value, maximum)?;
    if value.is_empty() {
        return Err(invalid());
    }
    Ok(value.to_vec())
}

pub(super) fn parent(path: &[u8]) -> &[u8] {
    path.iter()
        .rposition(|byte| *byte == b'/')
        .map_or(b"".as_slice(), |index| &path[..index])
}

pub(super) fn ancestors(path: &[u8]) -> impl Iterator<Item = &[u8]> {
    path.iter()
        .enumerate()
        .filter_map(|(index, byte)| (*byte == b'/').then_some(&path[..index]))
}

pub(super) fn basename(path: &[u8]) -> &[u8] {
    path.iter()
        .rposition(|byte| *byte == b'/')
        .map_or(path, |index| &path[index + 1..])
}

pub(super) fn child(parent: &[u8], name: &[u8]) -> GuestPath {
    let mut path = Vec::with_capacity(parent.len() + usize::from(!parent.is_empty()) + name.len());
    path.extend_from_slice(parent);
    if !parent.is_empty() {
        path.push(b'/');
    }
    path.extend_from_slice(name);
    path
}

fn validate_bytes(value: &[u8], maximum: u32) -> Result<(), NormalizeError> {
    let maximum = usize::try_from(maximum).map_err(|_| limit())?;
    if value.len() > maximum || value.contains(&0) {
        return Err(if value.len() > maximum {
            limit()
        } else {
            invalid()
        });
    }
    Ok(())
}

const fn invalid() -> NormalizeError {
    NormalizeError::new(NormalizePhase::ApplyLayer, NormalizeErrorKind::InvalidInput)
}

const fn limit() -> NormalizeError {
    NormalizeError::new(
        NormalizePhase::ApplyLayer,
        NormalizeErrorKind::LimitExceeded,
    )
}
