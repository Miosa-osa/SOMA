use std::collections::BTreeMap;

use soma::OciDigest;

use super::{
    entry::{GuestPath, Metadata, PlannedNode},
    layer::{LayerPlan, Whiteout},
    tree_model::{self, FileNode, Node, TreeStats},
};
use crate::{NormalizeError, NormalizeErrorKind, NormalizePhase, RootfsLimits};

mod hardlinks;
mod mutation;
#[cfg(test)]
mod tests;

pub(super) struct Tree {
    pub(super) entries: BTreeMap<GuestPath, Node>,
    next_inode: u64,
    maximum_entries: usize,
    maximum_metadata_bytes: u64,
    metadata_bytes: u64,
}

impl Tree {
    pub(super) fn new(limits: RootfsLimits) -> Result<Self, NormalizeError> {
        Self::with_limits(limits)
    }

    pub(super) fn apply(&mut self, plan: LayerPlan) -> Result<(), NormalizeError> {
        validate_addition_hierarchy(&plan.additions)?;
        for whiteout in &plan.whiteouts {
            let directory = match whiteout {
                Whiteout::Remove(path) => super::entry::parent(path),
                Whiteout::Opaque(path) => path,
            };
            self.ensure_whiteout_parent(directory, &plan.additions)?;
        }
        for whiteout in plan.whiteouts {
            match whiteout {
                Whiteout::Remove(path) => {
                    if path.is_empty() {
                        return Err(invalid());
                    }
                    self.remove_subtree(&path)?;
                }
                Whiteout::Opaque(path) => self.make_opaque(&path)?,
            }
        }
        for (path, planned) in &plan.additions {
            if let PlannedNode::Directory(metadata) = planned {
                self.put_directory(path, metadata.clone())?;
            }
        }
        for (path, planned) in &plan.additions {
            match planned {
                PlannedNode::Regular {
                    metadata,
                    digest,
                    size,
                } => self.put_regular(path, metadata.clone(), digest.clone(), *size)?,
                PlannedNode::Symlink { metadata, target } => {
                    self.put_node(
                        path,
                        Node::Symlink {
                            metadata: metadata.clone(),
                            target: target.clone(),
                        },
                    )?;
                }
                PlannedNode::Fifo(metadata) => {
                    self.put_node(path, Node::Fifo(metadata.clone()))?;
                }
                PlannedNode::Directory(_) | PlannedNode::Hardlink { .. } => {}
            }
        }
        self.put_hardlinks(&plan.additions)?;
        Ok(())
    }

    pub(super) fn stats(&self) -> Result<TreeStats, NormalizeError> {
        tree_model::stats(&self.entries)
    }

    fn make_opaque(&mut self, path: &[u8]) -> Result<(), NormalizeError> {
        if let Some(node) = self.entries.get(path)
            && !matches!(node, Node::Directory(_))
        {
            return Err(invalid());
        }
        self.remove_descendants(path)?;
        Ok(())
    }

    fn put_regular(
        &mut self,
        path: &[u8],
        metadata: Metadata,
        digest: OciDigest,
        size: u64,
    ) -> Result<(), NormalizeError> {
        let inode = self.next_inode;
        self.next_inode = self.next_inode.checked_add(1).ok_or_else(limit)?;
        self.put_node(
            path,
            Node::Regular(FileNode {
                inode,
                metadata,
                digest,
                size,
            }),
        )
    }
}

fn validate_addition_hierarchy(
    additions: &BTreeMap<GuestPath, PlannedNode>,
) -> Result<(), NormalizeError> {
    for path in additions.keys() {
        for parent in super::entry::ancestors(path) {
            if additions
                .get(parent)
                .is_some_and(|node| !matches!(node, PlannedNode::Directory(_)))
            {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

fn is_descendant(candidate: &[u8], parent: &[u8]) -> bool {
    if parent.is_empty() {
        return !candidate.is_empty();
    }
    candidate
        .strip_prefix(parent)
        .is_some_and(|suffix| suffix.starts_with(b"/"))
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
