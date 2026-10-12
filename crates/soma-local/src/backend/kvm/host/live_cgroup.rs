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
//!
//! Leaving the unit also leaves its accounting. The unit's memory and pids limits are shared by
//! everything in it, so they follow the machines as a limit on the delegated subtree's root, set
//! once when the unit starts (`soma-live-cgroup.sh`), never as a copy on each leaf: a copy per
//! machine would turn one shared cap into one cap each. The leaf only has to be accounted, which it
//! is when the subtree enables the memory and pids controllers.

use std::{
    fs::File,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use soma::InstanceId;
use soma_jail::CgroupLeaf;

use super::{
    super::resize::{CGROUP_ROOT, sweep_finished_leaves},
    wire::LiveCgroup,
};

/// How many times joining retries a step that a concurrent change to the subtree can undo.
const ATTEMPTS: usize = 5;
/// The pause between attempts.
const RETRY_PAUSE: Duration = Duration::from_millis(5);

/// What joining did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Joined {
    /// This host is in its own leaf, outside the unit's control group.
    Moved(Accounting),
    /// No delegated subtree is configured, so the host stays where it started.
    NotConfigured,
    /// A subtree is configured but the move failed; the host stays where it started.
    Failed,
}

/// Whether the leaf is accounted for memory and pids.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Accounting {
    /// The subtree enables the memory and pids controllers, so the leaf's usage is counted under
    /// the root's limit.
    Accounted,
    /// A controller is not enabled in the subtree; the machine moved but is not counted.
    Unaccounted,
}

impl Joined {
    /// What the host tells its parent, or nothing when no subtree is configured.
    pub(super) fn report(self) -> Option<LiveCgroup> {
        match self {
            Self::NotConfigured => None,
            Self::Failed => Some(LiveCgroup {
                joined: false,
                accounted: false,
            }),
            Self::Moved(accounting) => Some(LiveCgroup {
                joined: true,
                accounted: accounting == Accounting::Accounted,
            }),
        }
    }
}

/// Moves this process into the live leaf for `instance`, when the host is configured for one.
///
/// This is best effort by design: a host that cannot move still serves its machine, it only
/// loses the ability to outlive an API unit stop. The outcome travels to the parent in the launch
/// answer, because a machine host has no log of its own.
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

/// Runs `work` while holding an exclusive lock on the subtree's root directory.
///
/// Sweeping removes empty `-live` leaves, and a leaf is empty between its creation and the move
/// into it, so an unsynchronised sweep by another launch can remove the leaf a host is about to
/// join. Every sweep, create, and move therefore happens under this one lock. The lock is on the
/// directory itself because a cgroup directory cannot hold a lock file.
pub(in crate::backend::kvm) fn under_root_lock<T>(
    root: &Path,
    work: impl FnOnce() -> T,
) -> Option<T> {
    let directory = File::open(root).ok()?;
    directory.lock().ok()?;
    let result = work();
    drop(directory);
    Some(result)
}

fn join_under(root: &Path, instance: &InstanceId, pid: u32) -> Joined {
    let joined = under_root_lock(root, || {
        sweep_finished_leaves(root);
        let leaf_name = leaf_name(instance);
        for attempt in 0..ATTEMPTS {
            if attempt > 0 {
                thread::sleep(RETRY_PAUSE);
            }
            let Ok(leaf) = CgroupLeaf::create_cpu_only(root, &leaf_name) else {
                continue;
            };
            if leaf.attach(pid).is_ok() {
                return true;
            }
        }
        false
    });
    if joined != Some(true) {
        return Joined::Failed;
    }
    let leaf = root.join(leaf_name(instance));
    if leaf.join("memory.current").exists() && leaf.join("pids.current").exists() {
        Joined::Moved(Accounting::Accounted)
    } else {
        Joined::Moved(Accounting::Unaccounted)
    }
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

    #[test]
    fn a_failure_is_reported_to_the_parent_and_an_unconfigured_host_says_nothing() {
        assert_eq!(Joined::NotConfigured.report(), None);
        let failed = Joined::Failed.report().expect("reported");
        assert!(!failed.joined);
        let counted = Joined::Moved(Accounting::Accounted)
            .report()
            .expect("reported");
        assert!(counted.joined && counted.accounted);
        let bare = Joined::Moved(Accounting::Unaccounted)
            .report()
            .expect("reported");
        assert!(bare.joined && !bare.accounted);
    }

    /// The race the lock exists for, on a real cgroup2 root: many hosts creating and joining while
    /// others sweep. Every join must succeed. Run it in the privileged container with
    /// `SOMA_TEST_CGROUP_ROOT` naming a delegated cgroup2 directory that has `cpu` enabled.
    #[test]
    #[ignore = "requires a delegated cgroup2 subtree: set SOMA_TEST_CGROUP_ROOT"]
    fn concurrent_sweeps_and_joins_on_a_real_cgroup_never_lose_a_join() {
        let root = PathBuf::from(
            std::env::var_os("SOMA_TEST_CGROUP_ROOT").expect("SOMA_TEST_CGROUP_ROOT"),
        );
        let joiners: Vec<_> = (0..64_u32)
            .map(|index| {
                let root = root.clone();
                thread::spawn(move || {
                    // Each joiner is a real child process so that moving it is real; the sweep
                    // that races it runs on the other threads.
                    let mut child = std::process::Command::new("sleep")
                        .arg("30")
                        .spawn()
                        .expect("a process to move");
                    let id = format!("2222222222222222222222222{index:07x}");
                    let instance = InstanceId::new(&id).expect("instance");
                    let outcome = join_under(&root, &instance, child.id());
                    let moved = matches!(outcome, Joined::Moved(_));
                    child.kill().expect("stop the child");
                    child.wait().expect("reap the child");
                    (moved, outcome)
                })
            })
            .collect();
        let sweepers: Vec<_> = (0..8)
            .map(|_| {
                let root = root.clone();
                thread::spawn(move || {
                    for _ in 0..200 {
                        under_root_lock(&root, || sweep_finished_leaves(&root));
                    }
                })
            })
            .collect();
        for sweeper in sweepers {
            sweeper.join().expect("sweeper");
        }
        for joiner in joiners {
            let (moved, outcome) = joiner.join().expect("joiner");
            assert!(moved, "a join was lost to a concurrent sweep: {outcome:?}");
        }
        sweep_finished_leaves(&root);
    }
}
