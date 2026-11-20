//! The operations that change the in-memory guest's tree.

use soma::{FileAnswer, FileEntry, FileKind, FileMetadata, FileRefusal};

use super::{MODIFIED, MemoryGuest, Node};

impl MemoryGuest {
    pub(super) fn write(
        &mut self,
        path: &str,
        offset: u64,
        create: bool,
        shorten: bool,
        bytes: &[u8],
    ) -> Result<(), FileRefusal> {
        match self.nodes.get(path) {
            Some(Node::Directory { .. }) => return Err(FileRefusal::WrongKind),
            None if !create => return Err(FileRefusal::NotFound),
            None => {
                self.parent_ready(path)?;
                self.nodes.insert(
                    path.to_owned(),
                    Node::File {
                        bytes: Vec::new(),
                        mode: 0o644,
                    },
                );
            }
            Some(Node::File { .. }) => {}
        }
        let Some(Node::File { bytes: held, .. }) = self.nodes.get_mut(path) else {
            unreachable!("the file was just found or made");
        };
        let start = usize::try_from(offset).expect("a test offset");
        if held.len() < start + bytes.len() {
            held.resize(start + bytes.len(), 0);
        }
        held[start..start + bytes.len()].copy_from_slice(bytes);
        if shorten {
            held.truncate(start + bytes.len());
        }
        Ok(())
    }

    pub(super) fn make_directory(&mut self, path: &str) -> Result<FileAnswer, FileRefusal> {
        let mut built = String::new();
        for segment in path.split('/').filter(|segment| !segment.is_empty()) {
            built.push('/');
            built.push_str(segment);
            match self.nodes.get(&built) {
                Some(Node::Directory { .. }) => {}
                // `create_dir_all` meets a file: `EEXIST` at the end, `ENOTDIR` on the way.
                Some(Node::File { .. }) if built == path => return Err(FileRefusal::Exists),
                Some(Node::File { .. }) => return Err(FileRefusal::WrongKind),
                None => {
                    self.nodes
                        .insert(built.clone(), Node::Directory { mode: 0o755 });
                }
            }
        }
        Ok(FileAnswer::Done)
    }

    pub(super) fn list(&self, path: &str) -> Result<FileAnswer, FileRefusal> {
        match self.kind(path) {
            Some(FileKind::Directory) => {}
            Some(_) => return Err(FileRefusal::WrongKind),
            None => return Err(FileRefusal::NotFound),
        }
        let mut entries: Vec<FileEntry> = self
            .children(path)
            .into_iter()
            .map(|(name, node)| {
                let (kind, size, mode) = match node {
                    Node::Directory { mode } => (FileKind::Directory, 0, *mode),
                    Node::File { bytes, mode } => (FileKind::File, bytes.len() as u64, *mode),
                };
                FileEntry {
                    name: name.into_bytes(),
                    kind,
                    metadata: (!self.level_one).then_some(FileMetadata {
                        size,
                        mode,
                        modified: MODIFIED,
                    }),
                }
            })
            .collect();
        let limit = self.listing_limit.unwrap_or(usize::MAX);
        let more = entries.len() > limit;
        entries.truncate(limit);
        Ok(FileAnswer::Listed { entries, more })
    }

    pub(super) fn remove(
        &mut self,
        path: &str,
        recursive: bool,
    ) -> Result<FileAnswer, FileRefusal> {
        match self.kind(path) {
            None => Err(FileRefusal::NotFound),
            Some(FileKind::Directory) if !recursive && !self.children(path).is_empty() => {
                Err(FileRefusal::NotEmpty)
            }
            Some(_) => {
                let below = format!("{path}/");
                self.nodes
                    .retain(|stored, _| stored != path && !stored.starts_with(&below));
                Ok(FileAnswer::Done)
            }
        }
    }

    pub(super) fn rename(&mut self, from: &str, to: &str) -> Result<FileAnswer, FileRefusal> {
        if let Some(refusal) = self.refuse_renames {
            return Err(refusal);
        }
        let moving = self.kind(from).ok_or(FileRefusal::NotFound)?;
        self.parent_ready(to)?;
        match (moving, self.kind(to)) {
            (FileKind::File, Some(FileKind::Directory))
            | (FileKind::Directory, Some(FileKind::File)) => return Err(FileRefusal::WrongKind),
            (FileKind::Directory, Some(FileKind::Directory)) if !self.children(to).is_empty() => {
                return Err(FileRefusal::NotEmpty);
            }
            _ => {}
        }
        let below = format!("{from}/");
        let moved: Vec<(String, Node)> = self
            .nodes
            .iter()
            .filter(|(stored, _)| stored.as_str() == from || stored.starts_with(&below))
            .map(|(stored, node)| (format!("{to}{}", &stored[from.len()..]), node.clone()))
            .collect();
        self.nodes
            .retain(|stored, _| stored != from && !stored.starts_with(&below) && stored != to);
        self.nodes.extend(moved);
        Ok(FileAnswer::Done)
    }
}
