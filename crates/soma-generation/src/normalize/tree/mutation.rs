use std::{
    collections::BTreeMap,
    ops::Bound::{Excluded, Unbounded},
};

use super::{Tree, invalid, limit};
use crate::{NormalizeError, RootfsLimits};

use crate::normalize::{
    entry::{GuestPath, Metadata, PlannedNode},
    tree_model::Node,
};

impl Tree {
    pub(super) fn with_limits(limits: RootfsLimits) -> Result<Self, NormalizeError> {
        let maximum_entries = usize::try_from(limits.max_entries).map_err(|_| limit())?;
        if maximum_entries == 0 {
            return Err(limit());
        }
        let mut tree = Self {
            entries: BTreeMap::new(),
            next_inode: 1,
            maximum_entries,
            maximum_metadata_bytes: limits.max_metadata_bytes,
            metadata_bytes: 0,
        };
        tree.insert(Vec::new(), Node::Directory(Metadata::implicit_directory()))?;
        Ok(tree)
    }

    pub(super) fn put_directory(
        &mut self,
        path: &[u8],
        metadata: Metadata,
    ) -> Result<(), NormalizeError> {
        if !path.is_empty() {
            self.ensure_parents(path)?;
        }
        self.insert(path.to_vec(), Node::Directory(metadata))
    }

    pub(super) fn put_node(&mut self, path: &[u8], node: Node) -> Result<(), NormalizeError> {
        if path.is_empty() {
            return Err(invalid());
        }
        self.ensure_parents(path)?;
        if matches!(self.entries.get(path), Some(Node::Directory(_))) {
            self.remove_descendants(path)?;
        }
        self.insert(path.to_vec(), node)
    }

    pub(super) fn ensure_parents(&mut self, path: &[u8]) -> Result<(), NormalizeError> {
        let mut parent = Vec::new();
        let components: Vec<_> = path.split(|byte| *byte == b'/').collect();
        for component in &components[..components.len().saturating_sub(1)] {
            if !parent.is_empty() {
                parent.push(b'/');
            }
            parent.extend_from_slice(component);
            match self.entries.get(&parent) {
                Some(Node::Directory(_)) => {}
                Some(_) => return Err(invalid()),
                None => self.insert(
                    parent.clone(),
                    Node::Directory(Metadata::implicit_directory()),
                )?,
            }
        }
        Ok(())
    }

    pub(super) fn ensure_whiteout_parent(
        &mut self,
        path: &[u8],
        additions: &BTreeMap<GuestPath, PlannedNode>,
    ) -> Result<(), NormalizeError> {
        if path.is_empty() {
            return Ok(());
        }
        let mut directory = Vec::new();
        for component in path.split(|byte| *byte == b'/') {
            if !directory.is_empty() {
                directory.push(b'/');
            }
            directory.extend_from_slice(component);
            match self.entries.get(&directory) {
                Some(Node::Directory(_)) => {}
                Some(_) => {
                    let Some(PlannedNode::Directory(metadata)) = additions.get(&directory) else {
                        return Err(invalid());
                    };
                    self.insert(directory.clone(), Node::Directory(metadata.clone()))?;
                }
                None => {
                    let metadata = additions
                        .get(&directory)
                        .and_then(|node| match node {
                            PlannedNode::Directory(metadata) => Some(metadata.clone()),
                            _ => None,
                        })
                        .unwrap_or_else(Metadata::implicit_directory);
                    self.insert(directory.clone(), Node::Directory(metadata))?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn remove_subtree(&mut self, path: &[u8]) -> Result<(), NormalizeError> {
        self.remove_exact(path)?;
        self.remove_descendants(path)
    }

    pub(super) fn remove_descendants(&mut self, path: &[u8]) -> Result<(), NormalizeError> {
        let bounds = (!path.is_empty()).then(|| descendant_bounds(path));
        loop {
            let next = if let Some((start, end)) = &bounds {
                self.entries.range(start.clone()..end.clone()).next()
            } else {
                self.entries.range((Excluded(Vec::new()), Unbounded)).next()
            }
            .map(|(candidate, _)| candidate.clone());
            let Some(candidate) = next else {
                return Ok(());
            };
            self.remove_exact(&candidate)?;
        }
    }

    fn insert(&mut self, path: GuestPath, node: Node) -> Result<(), NormalizeError> {
        let old_cost = self
            .entries
            .get(&path)
            .map_or(Ok(0), |old| metadata_cost(&path, old))?;
        let new_cost = metadata_cost(&path, &node)?;
        let entry_count = if self.entries.contains_key(&path) {
            self.entries.len()
        } else {
            self.entries.len().checked_add(1).ok_or_else(limit)?
        };
        let metadata_bytes = self
            .metadata_bytes
            .checked_sub(old_cost)
            .and_then(|bytes| bytes.checked_add(new_cost))
            .ok_or_else(limit)?;
        if entry_count > self.maximum_entries || metadata_bytes > self.maximum_metadata_bytes {
            return Err(limit());
        }
        self.entries.insert(path, node);
        self.metadata_bytes = metadata_bytes;
        Ok(())
    }

    fn remove_exact(&mut self, path: &[u8]) -> Result<(), NormalizeError> {
        if let Some(node) = self.entries.remove(path) {
            self.metadata_bytes = self
                .metadata_bytes
                .checked_sub(metadata_cost(path, &node)?)
                .ok_or_else(limit)?;
        }
        Ok(())
    }
}

fn descendant_bounds(path: &[u8]) -> (GuestPath, GuestPath) {
    let mut start = path.to_vec();
    start.push(b'/');
    let mut end = path.to_vec();
    end.push(b'0');
    (start, end)
}

fn metadata_cost(path: &[u8], node: &Node) -> Result<u64, NormalizeError> {
    let link = match node {
        Node::Symlink { target, .. } => target.len(),
        Node::Directory(_) | Node::Regular(_) | Node::Fifo(_) => 0,
    };
    let bytes = path.len().checked_add(link).ok_or_else(limit)?;
    u64::try_from(bytes).map_err(|_| limit())
}
