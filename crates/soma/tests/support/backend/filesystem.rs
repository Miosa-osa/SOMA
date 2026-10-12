//! Flat in-memory guest filesystem used by the shared backend fixture.

use std::collections::BTreeMap;

use soma::{FileAnswer, FileEntry, FileKind, FileOperation, FileRefusal};

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
            let entries = files
                .keys()
                .filter_map(|stored| stored.strip_prefix(prefix.as_slice()))
                .filter(|name| !name.is_empty() && !name.contains(&b'/'))
                .map(|name| FileEntry {
                    name: name.to_vec(),
                    kind: FileKind::File,
                })
                .collect();
            FileAnswer::Listed {
                entries,
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
