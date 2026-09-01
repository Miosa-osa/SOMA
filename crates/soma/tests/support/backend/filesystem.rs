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
    }
}
