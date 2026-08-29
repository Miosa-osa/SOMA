use std::collections::{BTreeMap, BTreeSet};

use super::{
    artifacts::Sha256Digest,
    error::{CompileError, CompileErrorKind, CompilePhase},
};

#[cfg(test)]
mod tests;

const MAGIC: &[u8; 8] = b"SOMARFS\0";
const FORMAT_VERSION: u16 = 1;
const POLICY_VERSION: u16 = 1;
const SUPPORTED_MODE_MASK: u32 = 0o7777;

/// Explicit bounds for decoding one canonical tree manifest into a stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TreeBounds {
    /// Maximum logical entries including the root directory.
    pub max_entries: u32,
    /// Maximum bytes in one normalized path.
    pub max_path_bytes: u32,
    /// Maximum bytes in one symbolic-link target.
    pub max_link_bytes: u32,
    /// Maximum aggregate path and link bytes.
    pub max_metadata_bytes: u64,
    /// Maximum bytes in one regular file.
    pub max_file_bytes: u64,
    /// Maximum aggregate logical regular-file bytes.
    pub max_content_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TreeNode {
    Directory,
    Regular { size: u64, digest: Sha256Digest },
    Symlink { target: Vec<u8> },
    Hardlink { anchor: Vec<u8> },
    Fifo,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TreeEntry {
    pub(crate) path: Vec<u8>,
    pub(crate) mode: u32,
    pub(crate) uid: u32,
    pub(crate) gid: u32,
    pub(crate) mtime: u64,
    pub(crate) node: TreeNode,
}

/// A bounded hostile decoder that yields canonical tree entries in raw path-byte order.
pub(crate) struct TreeDecoder<'a> {
    bytes: &'a [u8],
    offset: usize,
    declared: u32,
    produced: u32,
    previous: Option<Vec<u8>>,
    directories: BTreeSet<Vec<u8>>,
    anchors: BTreeMap<Vec<u8>, (u64, Sha256Digest)>,
    bounds: TreeBounds,
    metadata_bytes: u64,
    content_bytes: u64,
    failed: bool,
}

impl<'a> TreeDecoder<'a> {
    pub(crate) fn new(bytes: &'a [u8], bounds: TreeBounds) -> Result<Self, CompileError> {
        let mut decoder = Self {
            bytes,
            offset: 0,
            declared: 0,
            produced: 0,
            previous: None,
            directories: BTreeSet::new(),
            anchors: BTreeMap::new(),
            bounds,
            metadata_bytes: 0,
            content_bytes: 0,
            failed: false,
        };
        if decoder.consume(8)? != MAGIC
            || decoder.u16()? != FORMAT_VERSION
            || decoder.u16()? != POLICY_VERSION
        {
            return Err(invalid());
        }
        decoder.declared = decoder.u32()?;
        if decoder.declared == 0 {
            return Err(invalid());
        }
        if decoder.declared > bounds.max_entries {
            return Err(limit());
        }
        Ok(decoder)
    }

    pub(crate) const fn declared_entries(&self) -> u32 {
        self.declared
    }

    /// Requires that every declared entry was consumed and no bytes remain.
    pub(crate) fn finish(self) -> Result<TreeSummary, CompileError> {
        if self.failed || self.produced != self.declared || self.offset != self.bytes.len() {
            return Err(invalid());
        }
        Ok(TreeSummary {
            entry_count: self.produced,
            content_bytes: self.content_bytes,
        })
    }

    fn next_entry(&mut self) -> Result<TreeEntry, CompileError> {
        let path = self.sized(self.bounds.max_path_bytes)?;
        self.charge_metadata(path.len())?;
        self.validate_path(&path)?;
        let kind = self.u8()?;
        let mode = self.u32()?;
        let uid = self.u32()?;
        let gid = self.u32()?;
        let mtime = self.u64()?;
        if self.u32()? != 0 || mode & !SUPPORTED_MODE_MASK != 0 {
            return Err(invalid());
        }
        let node = match kind {
            1 => {
                self.directories.insert(path.clone());
                TreeNode::Directory
            }
            2 => self.regular(&path)?,
            3 => {
                let target = self.sized(self.bounds.max_link_bytes)?;
                self.charge_metadata(target.len())?;
                if target.is_empty() || target.contains(&0) {
                    return Err(invalid());
                }
                TreeNode::Symlink { target }
            }
            4 => TreeNode::Fifo,
            5 => {
                let anchor = self.sized(self.bounds.max_path_bytes)?;
                self.charge_metadata(anchor.len())?;
                if anchor.as_slice() >= path.as_slice() || !self.anchors.contains_key(&anchor) {
                    return Err(invalid());
                }
                TreeNode::Hardlink { anchor }
            }
            _ => {
                return Err(CompileError::new(
                    CompilePhase::DecodeTree,
                    CompileErrorKind::Unsupported,
                ));
            }
        };
        if path.is_empty() && node != TreeNode::Directory {
            return Err(invalid());
        }
        self.previous = Some(path.clone());
        self.produced = self.produced.checked_add(1).ok_or_else(limit)?;
        Ok(TreeEntry {
            path,
            mode,
            uid,
            gid,
            mtime,
            node,
        })
    }

    fn regular(&mut self, path: &[u8]) -> Result<TreeNode, CompileError> {
        let size = self.u64()?;
        let mut digest = [0_u8; 32];
        digest.copy_from_slice(self.consume(32)?);
        if size > self.bounds.max_file_bytes {
            return Err(limit());
        }
        self.content_bytes = self.content_bytes.checked_add(size).ok_or_else(limit)?;
        if self.content_bytes > self.bounds.max_content_bytes {
            return Err(limit());
        }
        let digest = Sha256Digest::from_bytes(digest);
        self.anchors.insert(path.to_vec(), (size, digest));
        Ok(TreeNode::Regular { size, digest })
    }

    fn validate_path(&self, path: &[u8]) -> Result<(), CompileError> {
        match &self.previous {
            None if !path.is_empty() => return Err(invalid()),
            Some(previous) if path <= previous.as_slice() => return Err(invalid()),
            _ => {}
        }
        if path.is_empty() {
            return Ok(());
        }
        if path.contains(&0) || path.starts_with(b"/") || path.ends_with(b"/") {
            return Err(invalid());
        }
        if path
            .split(|byte| *byte == b'/')
            .any(|component| matches!(component, b"" | b"." | b".."))
        {
            return Err(invalid());
        }
        let parent = path
            .iter()
            .rposition(|byte| *byte == b'/')
            .map_or(b"".as_slice(), |index| &path[..index]);
        if !self.directories.contains(parent) {
            return Err(invalid());
        }
        Ok(())
    }

    fn charge_metadata(&mut self, count: usize) -> Result<(), CompileError> {
        let count = u64::try_from(count).map_err(|_| limit())?;
        self.metadata_bytes = self.metadata_bytes.checked_add(count).ok_or_else(limit)?;
        if self.metadata_bytes > self.bounds.max_metadata_bytes {
            return Err(limit());
        }
        Ok(())
    }

    fn sized(&mut self, maximum: u32) -> Result<Vec<u8>, CompileError> {
        let length = self.u32()?;
        if length > maximum {
            return Err(limit());
        }
        let length = usize::try_from(length).map_err(|_| limit())?;
        Ok(self.consume(length)?.to_vec())
    }

    fn consume(&mut self, count: usize) -> Result<&'a [u8], CompileError> {
        let end = self.offset.checked_add(count).ok_or_else(invalid)?;
        let value = self.bytes.get(self.offset..end).ok_or_else(invalid)?;
        self.offset = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, CompileError> {
        Ok(self.consume(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, CompileError> {
        Ok(u16::from_be_bytes(
            self.consume(2)?.try_into().map_err(|_| invalid())?,
        ))
    }

    fn u32(&mut self) -> Result<u32, CompileError> {
        Ok(u32::from_be_bytes(
            self.consume(4)?.try_into().map_err(|_| invalid())?,
        ))
    }

    fn u64(&mut self) -> Result<u64, CompileError> {
        Ok(u64::from_be_bytes(
            self.consume(8)?.try_into().map_err(|_| invalid())?,
        ))
    }
}

impl Iterator for TreeDecoder<'_> {
    type Item = Result<TreeEntry, CompileError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.produced == self.declared {
            return None;
        }
        let entry = self.next_entry();
        if entry.is_err() {
            self.failed = true;
        }
        Some(entry)
    }
}

/// The verified totals of one fully consumed tree stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TreeSummary {
    pub(crate) entry_count: u32,
    pub(crate) content_bytes: u64,
}

const fn invalid() -> CompileError {
    CompileError::new(CompilePhase::DecodeTree, CompileErrorKind::InvalidInput)
}

const fn limit() -> CompileError {
    CompileError::new(CompilePhase::DecodeTree, CompileErrorKind::LimitExceeded)
}
