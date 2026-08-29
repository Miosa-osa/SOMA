use std::io::Read;

use tar::EntryType;

use super::entry::{self, Metadata, PlannedNode};
use crate::{
    ImportError, ImportErrorKind, ImportPhase, NormalizeError, NormalizeErrorKind, NormalizePhase,
    RootfsLimits, store::Store,
};

pub(crate) const CONTENT_MEDIA_TYPE: &str = "application/vnd.soma.rootfs.file.v1";

pub(super) fn parse<R: Read>(
    entry: &mut tar::Entry<'_, R>,
    store: &Store,
    limits: RootfsLimits,
    kind: EntryType,
    size: u64,
    link: Option<&[u8]>,
) -> Result<PlannedNode, NormalizeError> {
    if kind.is_file() {
        require_no_link(link)?;
        if size > limits.max_file_bytes {
            return Err(limit());
        }
        let metadata = metadata(entry.header())?;
        let content = store
            .put_content(
                entry,
                size,
                limits.max_file_bytes,
                CONTENT_MEDIA_TYPE,
                ImportPhase::VerifyLayer,
            )
            .map_err(map_content_error)?;
        Ok(PlannedNode::Regular {
            metadata,
            digest: content.digest,
            size,
        })
    } else if kind.is_dir() {
        require_empty(size, link)?;
        Ok(PlannedNode::Directory(metadata(entry.header())?))
    } else if kind.is_symlink() {
        require_empty(size, None)?;
        let target = link.ok_or_else(invalid)?;
        Ok(PlannedNode::Symlink {
            metadata: metadata(entry.header())?,
            target: entry::validate_link(target, limits.max_path_bytes)?,
        })
    } else if kind.is_hard_link() {
        require_empty(size, None)?;
        let target = link.ok_or_else(invalid)?;
        Ok(PlannedNode::Hardlink {
            target: entry::normalize_path(target, limits.max_path_bytes)?,
        })
    } else if kind.is_fifo() {
        require_empty(size, link)?;
        Ok(PlannedNode::Fifo(metadata(entry.header())?))
    } else {
        Err(unsupported())
    }
}

fn metadata(header: &tar::Header) -> Result<Metadata, NormalizeError> {
    let mode = header.mode().map_err(|_| invalid())?;
    if mode & !0o7777 != 0 {
        return Err(unsupported());
    }
    Ok(Metadata {
        mode,
        uid: u32::try_from(header.uid().map_err(|_| invalid())?).map_err(|_| unsupported())?,
        gid: u32::try_from(header.gid().map_err(|_| invalid())?).map_err(|_| unsupported())?,
        mtime: header.mtime().map_err(|_| invalid())?,
    })
}

fn require_empty(size: u64, link: Option<&[u8]>) -> Result<(), NormalizeError> {
    if size != 0 || link.is_some() {
        return Err(invalid());
    }
    Ok(())
}

fn require_no_link(link: Option<&[u8]>) -> Result<(), NormalizeError> {
    if link.is_some() {
        return Err(invalid());
    }
    Ok(())
}

fn map_content_error(error: ImportError) -> NormalizeError {
    let kind = match error.kind() {
        ImportErrorKind::LimitExceeded => NormalizeErrorKind::LimitExceeded,
        ImportErrorKind::Integrity => NormalizeErrorKind::Integrity,
        ImportErrorKind::StoreConflict => NormalizeErrorKind::StoreConflict,
        _ => NormalizeErrorKind::Io,
    };
    NormalizeError::new(NormalizePhase::Publish, kind)
}

const fn invalid() -> NormalizeError {
    NormalizeError::new(NormalizePhase::ApplyLayer, NormalizeErrorKind::InvalidInput)
}

const fn unsupported() -> NormalizeError {
    NormalizeError::new(NormalizePhase::ApplyLayer, NormalizeErrorKind::Unsupported)
}

const fn limit() -> NormalizeError {
    NormalizeError::new(
        NormalizePhase::ApplyLayer,
        NormalizeErrorKind::LimitExceeded,
    )
}
