//! Keeping a claimed machine alive when the process that started its host is restarted.
//!
//! A machine host is a child of the hosted API. Its machine is addressed through the Instance
//! socket, which any later API process can open, so nothing about the machine needs the parent
//! (ADR 0042). What can still end it is the service manager: stopping the API unit sends the unit's
//! control group a kill, and a host that is still in that group goes with it.
//!
//! A host that has claimed an Instance therefore moves itself out of that group, into a leaf of the
//! delegated subtree the operator named for live machines, before it launches anything. The leaf
//! is the same one a live CPU resize uses, so a later resize finds it. A sterile host that has
//! claimed nothing stays where it is and ends with the unit, which is what an unused pool member
//! should do.

use std::path::{Path, PathBuf};

use soma::InstanceId;
use soma_jail::CgroupLeaf;

use super::super::resize::{CGROUP_ROOT, sweep_finished_leaves};

/// How many times joining retries when a concurrent sweep removes the leaf between its creation and the move.
const ATTEMPTS: usize = 3;

/// What joining did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Joined {
    /// This host is in its own leaf, outside the unit's control group.
    Moved,
    /// No delegated subtree is configured, so the host stays where it started.
    NotConfigured,
    /// A subtree is configured but the move failed; the host stays where it started.
    Failed,
}

/// Moves this process into the live leaf for `instance`, when the host is configured for one.
///
/// This is best effort by design: a host that cannot move still serves its machine, it only
/// loses the ability to outlive an API unit stop. The caller reports nothing on failure because
/// a machine host has no log of its own.
pub(super) fn join(instance: &InstanceId) -> Joined {
    match std::env::var_os(CGROUP_ROOT).map(PathBuf::from) {
        Some(root) => join_under(&root, instance, std::process::id()),
        None => Joined::NotConfigured,
    }
}

/// The leaf name a live machine uses; resize looks for the same one.
pub(super) fn leaf_name(instance: &InstanceId) -> String {
    format!("soma-{}-live", instance.as_str())
}

fn join_under(root: &Path, instance: &InstanceId, pid: u32) -> Joined {
    sweep_finished_leaves(root);
    for _ in 0..ATTEMPTS {
        let Ok(leaf) = CgroupLeaf::create_cpu_only(root, &leaf_name(instance)) else {
            return Joined::Failed;
        };
        if leaf.attach(pid).is_ok() {
            return Joined::Moved;
        }
    }
    Joined::Failed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_leaf_is_the_one_a_live_resize_creates() {
        let instance = InstanceId::new("22222222222222222222222222222222").expect("instance");
        assert_eq!(
            leaf_name(&instance),
            "soma-22222222222222222222222222222222-live"
        );
    }

    #[test]
    fn a_root_that_is_not_cgroup2_fails_without_moving_anything() {
        let root = tempfile::tempdir().expect("plain directory");
        let instance = InstanceId::new("22222222222222222222222222222222").expect("instance");
        assert_eq!(join_under(root.path(), &instance, 1), Joined::Failed);
    }
}
