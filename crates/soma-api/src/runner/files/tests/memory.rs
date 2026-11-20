//! An in-memory guest that answers file operations the way the guest agent's filesystem does.
//!
//! It is not a filesystem abstraction: it is the guest agent's observable behaviour on the cases
//! the file contract reaches, written down so the runner's sequences can be driven without a
//! machine. Each answer follows the errno the agent's own call would see on Linux, reduced to the
//! cause the protocol admits (`crates/soma-guest-agent/src/filesystem/failure.rs`).

use std::collections::BTreeMap;

use soma::{FileAnswer, FileKind, FileOperation, FileRefusal, ManagedFailure};

use crate::runner::files::guest::GuestFiles;

mod tree;

/// The modification time every node reports.
pub(crate) const MODIFIED: u64 = 1_790_960_645;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Node {
    Directory { mode: u32 },
    File { bytes: Vec<u8>, mode: u32 },
}

/// The guest's tree, by absolute path; `/` always exists.
#[derive(Debug, Default)]
pub(crate) struct MemoryGuest {
    pub(crate) nodes: BTreeMap<String, Node>,
    /// Plays a guest at API level 1: no stat, rename, mode, or ranged transfer, and a listing
    /// without sizes or times.
    pub(crate) level_one: bool,
    /// Refuses every rename with this cause, to drive a write that fails at its last step.
    pub(crate) refuse_renames: Option<FileRefusal>,
    /// The most entries one listing returns before it says more remain.
    pub(crate) listing_limit: Option<usize>,
    /// The facade failure every operation answers with, as an unreachable sandbox would.
    pub(crate) unreachable: Option<ManagedFailure>,
<<<<<<< HEAD
    /// How many more operations are answered before the sandbox becomes unreachable.
    pub(crate) answers_left: Option<usize>,
=======
>>>>>>> origin/r3/soma-unified
    /// Every operation performed, by name, in order.
    pub(crate) log: Vec<&'static str>,
}

impl MemoryGuest {
    pub(crate) fn file(&self, path: &str) -> Option<&[u8]> {
        match self.nodes.get(path) {
            Some(Node::File { bytes, .. }) => Some(bytes),
            _ => None,
        }
    }

    pub(crate) fn mode(&self, path: &str) -> Option<u32> {
        match self.nodes.get(path) {
            Some(Node::File { mode, .. } | Node::Directory { mode }) => Some(*mode),
            None => None,
        }
    }

    pub(crate) fn seed_file(&mut self, path: &str, bytes: &[u8]) {
        self.seed_directory(parent(path));
        self.nodes.insert(
            path.to_owned(),
            Node::File {
                bytes: bytes.to_vec(),
                mode: 0o644,
            },
        );
    }

    pub(crate) fn seed_directory(&mut self, path: &str) {
        let mut built = String::new();
        for segment in path.split('/').filter(|segment| !segment.is_empty()) {
            built.push('/');
            built.push_str(segment);
            self.nodes
                .entry(built.clone())
                .or_insert(Node::Directory { mode: 0o755 });
        }
    }

    pub(super) fn kind(&self, path: &str) -> Option<FileKind> {
        if path == "/" {
            return Some(FileKind::Directory);
        }
        self.nodes.get(path).map(|node| match node {
            Node::Directory { .. } => FileKind::Directory,
            Node::File { .. } => FileKind::File,
        })
    }

    /// `ENOENT` for a missing parent, `ENOTDIR` for a parent that is a file.
    pub(super) fn parent_ready(&self, path: &str) -> Result<(), FileRefusal> {
        match self.kind(parent(path)) {
            Some(FileKind::Directory) => Ok(()),
            Some(_) => Err(FileRefusal::WrongKind),
            None => Err(FileRefusal::NotFound),
        }
    }

    pub(super) fn children(&self, path: &str) -> Vec<(String, &Node)> {
        let prefix = if path == "/" {
            "/".to_owned()
        } else {
            format!("{path}/")
        };
        self.nodes
            .iter()
            .filter_map(|(stored, node)| {
                let name = stored.strip_prefix(&prefix)?;
                (!name.is_empty() && !name.contains('/')).then(|| (name.to_owned(), node))
            })
            .collect()
    }

    fn answer(&mut self, operation: &FileOperation) -> Result<FileAnswer, FileRefusal> {
        let ranged = matches!(
            operation,
            FileOperation::Stat { .. }
                | FileOperation::Rename { .. }
                | FileOperation::SetMode { .. }
                | FileOperation::ReadRange { .. }
                | FileOperation::WriteRange { .. }
        );
        if ranged && self.level_one {
            return Err(FileRefusal::Unsupported);
        }
        let path = text(operation.path());
        match operation {
            FileOperation::Read { .. } => {
                let bytes = self.readable(&path)?.to_vec();
                if bytes.len() > soma::MAX_FILE_BYTES {
                    return Err(FileRefusal::TooLarge);
                }
                Ok(FileAnswer::Read { bytes })
            }
            FileOperation::ReadRange { offset, length, .. } => {
                let bytes = self.readable(&path)?;
                let start = usize::try_from(*offset)
                    .unwrap_or(usize::MAX)
                    .min(bytes.len());
                let stop = start.saturating_add(*length as usize).min(bytes.len());
                Ok(FileAnswer::Range {
                    bytes: bytes[start..stop].to_vec(),
                    end: stop >= bytes.len(),
                })
            }
            FileOperation::Write { bytes, .. } => {
                self.write(&path, 0, true, true, bytes)?;
                Ok(FileAnswer::Written {
                    bytes: bytes.len() as u64,
                })
            }
            FileOperation::WriteRange {
                offset,
                create,
                shorten,
                bytes,
                ..
            } => {
                self.write(&path, *offset, *create, *shorten, bytes)?;
                Ok(FileAnswer::Written {
                    bytes: bytes.len() as u64,
                })
            }
            FileOperation::MakeDirectory { .. } => self.make_directory(&path),
            FileOperation::ReadDirectory { .. } => self.list(&path),
            FileOperation::Exists { .. } => Ok(FileAnswer::Status {
                kind: self.kind(&path),
            }),
            FileOperation::Stat { .. } => match self.nodes.get(&path) {
                Some(Node::File { bytes, mode }) => Ok(stat(FileKind::File, bytes.len(), *mode)),
                Some(Node::Directory { mode }) => Ok(stat(FileKind::Directory, 0, *mode)),
                None => Err(FileRefusal::NotFound),
            },
            FileOperation::SetMode { mode, .. } => match self.nodes.get_mut(&path) {
                Some(Node::File { mode: set, .. } | Node::Directory { mode: set }) => {
                    *set = *mode;
                    Ok(FileAnswer::Done)
                }
                None => Err(FileRefusal::NotFound),
            },
            FileOperation::Remove { recursive, .. } => self.remove(&path, *recursive),
            FileOperation::Rename { to, .. } => self.rename(&path, &text(to)),
        }
    }

    fn readable(&self, path: &str) -> Result<&[u8], FileRefusal> {
        match self.nodes.get(path) {
            Some(Node::File { bytes, .. }) => Ok(bytes),
            Some(Node::Directory { .. }) => Err(FileRefusal::WrongKind),
            None => Err(FileRefusal::NotFound),
        }
    }
}

impl GuestFiles for MemoryGuest {
    fn perform(&mut self, operation: FileOperation) -> Result<FileAnswer, ManagedFailure> {
        self.log.push(operation.name());
<<<<<<< HEAD
        if let Some(left) = self.answers_left.as_mut() {
            if *left == 0 {
                self.unreachable = Some(ManagedFailure::Backend(
                    soma::BackendFailureKind::Unavailable,
                ));
            } else {
                *left -= 1;
            }
        }
=======
>>>>>>> origin/r3/soma-unified
        if let Some(failure) = self.unreachable.clone() {
            return Err(failure);
        }
        Ok(self.answer(&operation).unwrap_or_else(FileAnswer::Refused))
    }
}

fn stat(kind: FileKind, size: usize, mode: u32) -> FileAnswer {
    FileAnswer::Stat {
        kind,
        size: size as u64,
        mode,
        modified: MODIFIED,
    }
}

fn parent(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) | None => "/",
        Some(index) => &path[..index],
    }
}

fn text(path: &[u8]) -> String {
    String::from_utf8(path.to_vec()).expect("test paths are UTF-8")
}
