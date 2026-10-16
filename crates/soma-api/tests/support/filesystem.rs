//! The in-memory filesystem the route double answers its file routes from.
//!
//! It lives apart from the double's own fields so the double stays inside the source budget; a
//! child module may read them, so the split costs nothing but a `use`.

use soma::{FileAnswer, FileEntry, FileKind, FileMetadata, FileOperation, FileRefusal};

use super::*;

/// The in-memory answer to one operation, deliberately flat: directories exist only as the
/// prefixes of the paths stored under them.
pub(super) fn answer_for(
    files: &mut BTreeMap<Vec<u8>, Vec<u8>>,
    operation: &FileOperation,
) -> FileAnswer {
    match operation {
        FileOperation::Read { path } => {
            files
                .get(path)
                .map_or(FileAnswer::Refused(FileRefusal::NotFound), |bytes| {
                    FileAnswer::Read {
                        bytes: bytes.clone(),
                    }
                })
        }
        FileOperation::Write { path, bytes } => {
            if path.starts_with(b"/readonly/") {
                return FileAnswer::Refused(FileRefusal::Denied);
            }
            files.insert(path.clone(), bytes.clone());
            FileAnswer::Written {
                bytes: bytes.len() as u64,
            }
        }
        FileOperation::MakeDirectory { .. } => FileAnswer::Done,
        FileOperation::ReadDirectory { path } => {
            let mut prefix = path.clone();
            if prefix.last() != Some(&b'/') {
                prefix.push(b'/');
            }
            FileAnswer::Listed {
                // A directory under /legacy/ plays a level 1 guest: names and kinds, no metadata.
                entries: files
                    .iter()
                    .filter_map(|(stored, bytes)| {
                        stored
                            .strip_prefix(prefix.as_slice())
                            .map(|name| (name, bytes))
                    })
                    .filter(|(name, _)| !name.is_empty() && !name.contains(&b'/'))
                    .map(|(name, bytes)| FileEntry {
                        name: name.to_vec(),
                        kind: FileKind::File,
                        metadata: (!prefix.starts_with(b"/legacy/")).then_some(FileMetadata {
                            size: bytes.len() as u64,
                            mode: 0o644,
                            modified: 1_760_000_000,
                        }),
                    })
                    .collect(),
                more: false,
            }
        }
        FileOperation::Exists { path } => FileAnswer::Status {
            kind: files.contains_key(path).then_some(FileKind::File),
        },
        FileOperation::Remove { path, .. } => {
            if files.remove(path).is_some() {
                FileAnswer::Done
            } else {
                FileAnswer::Refused(FileRefusal::NotFound)
            }
        }
        FileOperation::Stat { .. }
        | FileOperation::Rename { .. }
        | FileOperation::SetMode { .. }
        | FileOperation::ReadRange { .. }
        | FileOperation::WriteRange { .. } => level_two_answer(files, operation),
    }
}

/// The in-memory answer to the guest API level 2 operations.
fn level_two_answer(
    files: &mut BTreeMap<Vec<u8>, Vec<u8>>,
    operation: &FileOperation,
) -> FileAnswer {
    match operation {
        FileOperation::Stat { path } => {
            files
                .get(path)
                .map_or(FileAnswer::Refused(FileRefusal::NotFound), |bytes| {
                    FileAnswer::Stat {
                        kind: FileKind::File,
                        size: bytes.len() as u64,
                        mode: 0o644,
                        modified: 0,
                    }
                })
        }
        FileOperation::Rename { path, to } => match files.remove(path) {
            Some(bytes) => {
                files.insert(to.clone(), bytes);
                FileAnswer::Done
            }
            None => FileAnswer::Refused(FileRefusal::NotFound),
        },
        FileOperation::SetMode { path, .. } => {
            if files.contains_key(path) {
                FileAnswer::Done
            } else {
                FileAnswer::Refused(FileRefusal::NotFound)
            }
        }
        FileOperation::ReadRange {
            path,
            offset,
            length,
        } => files
            .get(path)
            .map_or(FileAnswer::Refused(FileRefusal::NotFound), |bytes| {
                let start = usize::try_from(*offset).map_or(bytes.len(), |o| o.min(bytes.len()));
                let stop = start.saturating_add(*length as usize).min(bytes.len());
                FileAnswer::Range {
                    bytes: bytes[start..stop].to_vec(),
                    end: stop == bytes.len(),
                }
            }),
        FileOperation::WriteRange {
            path,
            offset,
            create,
            shorten,
            bytes,
        } => {
            if !create && !files.contains_key(path) {
                return FileAnswer::Refused(FileRefusal::NotFound);
            }
            let file = files.entry(path.clone()).or_default();
            let start = usize::try_from(*offset).unwrap_or(usize::MAX);
            let stop = start + bytes.len();
            if file.len() < stop || *shorten {
                file.resize(stop, 0);
            }
            file[start..stop].copy_from_slice(bytes);
            FileAnswer::Written {
                bytes: bytes.len() as u64,
            }
        }
        _ => unreachable!("the caller routes only level 2 operations here"),
    }
}
