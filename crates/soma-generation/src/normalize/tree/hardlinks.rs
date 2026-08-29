use std::collections::{BTreeMap, VecDeque};

use super::{Tree, invalid, is_descendant};
use crate::{
    NormalizeError,
    normalize::entry::{self, GuestPath, PlannedNode},
    normalize::tree_model::Node,
};

impl Tree {
    pub(super) fn put_hardlinks(
        &mut self,
        additions: &BTreeMap<GuestPath, PlannedNode>,
    ) -> Result<(), NormalizeError> {
        let pending = pending_links(additions)?;
        let mut dependents = BTreeMap::<GuestPath, Vec<GuestPath>>::new();
        let mut ready = VecDeque::new();

        for (path, target) in &pending {
            if has_pending_ancestor(target, &pending) {
                return Err(invalid());
            }
            if pending.contains_key(target) {
                dependents
                    .entry(target.clone())
                    .or_default()
                    .push(path.clone());
            } else if let Some(Node::Regular(file)) = self.entries.get(target).cloned() {
                ready.push_back((path.clone(), file));
            } else {
                return Err(invalid());
            }
        }

        let expected = pending.len();
        let mut resolved = 0_usize;
        while let Some((path, file)) = ready.pop_front() {
            self.put_node(&path, Node::Regular(file.clone()))?;
            resolved = resolved.checked_add(1).ok_or_else(invalid)?;
            if let Some(paths) = dependents.remove(&path) {
                for dependent in paths {
                    ready.push_back((dependent, file.clone()));
                }
            }
        }
        if resolved != expected {
            return Err(invalid());
        }
        Ok(())
    }
}

fn pending_links(
    additions: &BTreeMap<GuestPath, PlannedNode>,
) -> Result<BTreeMap<GuestPath, GuestPath>, NormalizeError> {
    let mut pending = BTreeMap::new();
    for (path, node) in additions {
        if let PlannedNode::Hardlink { target } = node {
            if path.is_empty() || path == target || is_descendant(target, path) {
                return Err(invalid());
            }
            pending.insert(path.clone(), target.clone());
        }
    }
    Ok(pending)
}

fn has_pending_ancestor(target: &[u8], pending: &BTreeMap<GuestPath, GuestPath>) -> bool {
    entry::ancestors(target).any(|ancestor| pending.contains_key(ancestor))
}
