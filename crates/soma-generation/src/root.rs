use std::path::{Path, PathBuf};

use cap_fs_ext::{DirExt as _, ambient_authority};
#[cfg(windows)]
use cap_std::fs::MetadataExt as _;
use cap_std::fs::{Dir, Metadata};

use crate::{ImportError, ImportErrorKind, ImportPhase};

pub(crate) fn open_existing(
    path: &Path,
    phase: ImportPhase,
    rejected_kind: ImportErrorKind,
) -> Result<Dir, ImportError> {
    let name = path
        .file_name()
        .ok_or_else(|| ImportError::new(phase, rejected_kind))?;
    let parent_path = usable_parent(path);
    let parent = Dir::open_ambient_dir(&parent_path, ambient_authority())
        .map_err(|_| ImportError::new(phase, ImportErrorKind::Io))?;
    let entry = parent
        .symlink_metadata(name)
        .map_err(|_| ImportError::new(phase, rejected_kind))?;
    if is_link_like(&entry) {
        return Err(ImportError::new(phase, rejected_kind));
    }
    let root = parent
        .open_dir_nofollow(name)
        .map_err(|_| ImportError::new(phase, rejected_kind))?;
    let opened = root
        .dir_metadata()
        .map_err(|_| ImportError::new(phase, rejected_kind))?;
    if is_link_like(&opened) {
        return Err(ImportError::new(phase, rejected_kind));
    }
    Ok(root)
}

#[cfg(not(windows))]
fn is_link_like(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
fn is_link_like(metadata: &Metadata) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn usable_parent(path: &Path) -> PathBuf {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}
