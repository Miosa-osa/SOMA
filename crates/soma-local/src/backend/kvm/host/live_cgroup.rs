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
//! Leaving the unit also leaves its accounting, so the leaf takes over what the unit gave the
//! machine: when the subtree delegates the memory and pids controllers, the leaf is limited to the
//! unit's own `memory.max` and `pids.max` (a limit the unit does not set is not invented).

use std::{
    fs::{self, File},
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

/// Whether the leaf carries the unit's memory and pids limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Accounting {
    /// The controllers are delegated and every limit the unit sets was copied to the leaf.
    Carried,
    /// The memory or pids controller is not delegated here, or a limit could not be written; the
    /// machine moved, but the unit's limit does not follow it.
    NotCarried,
}

impl Joined {
    /// What the host tells its parent, or nothing when no subtree is configured.
    pub(super) fn report(self) -> Option<LiveCgroup> {
        match self {
            Self::NotConfigured => None,
            Self::Failed => Some(LiveCgroup {
                joined: false,
                limits_carried: false,
            }),
            Self::Moved(accounting) => Some(LiveCgroup {
                joined: true,
                limits_carried: accounting == Accounting::Carried,
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
    let unit = own_cgroup();
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
    let accounting = match unit {
        Some(unit) if carry_unit_limits(&unit, &root.join(leaf_name(instance))) => {
            Accounting::Carried
        }
        _ => Accounting::NotCarried,
    };
    Joined::Moved(accounting)
}

/// The cgroup directory this process is in, read before it moves.
fn own_cgroup() -> Option<PathBuf> {
    let text = fs::read_to_string("/proc/self/cgroup").ok()?;
    let relative = text.lines().find_map(|line| line.strip_prefix("0::"))?;
    Some(PathBuf::from("/sys/fs/cgroup").join(relative.trim_start_matches('/')))
}

/// Copies the unit's `memory.max` and `pids.max` onto the leaf. True when nothing the unit sets was
/// left behind, including when the unit sets nothing.
fn carry_unit_limits(unit: &Path, leaf: &Path) -> bool {
    ["memory.max", "pids.max"].iter().all(|file| {
        let wanted = fs::read_to_string(unit.join(file)).unwrap_or_default();
        let wanted = wanted.trim();
        // An absent or unlimited limit has nothing to carry.
        if wanted.is_empty() || wanted == "max" {
            return true;
        }
        fs::write(leaf.join(file), wanted).is_ok()
    })
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
        let carried = Joined::Moved(Accounting::Carried)
            .report()
            .expect("reported");
        assert!(carried.joined && carried.limits_carried);
        let bare = Joined::Moved(Accounting::NotCarried)
            .report()
            .expect("reported");
        assert!(bare.joined && !bare.limits_carried);
    }

    #[test]
    fn only_limits_the_unit_actually_sets_are_carried() {
        let unit = tempfile::tempdir().expect("unit");
        let leaf = tempfile::tempdir().expect("leaf");
        fs::write(unit.path().join("memory.max"), "max\n").expect("memory.max");
        fs::write(unit.path().join("pids.max"), "4096\n").expect("pids.max");
        fs::write(leaf.path().join("pids.max"), "max\n").expect("leaf pids.max");
        assert!(carry_unit_limits(unit.path(), leaf.path()));
        assert_eq!(
            fs::read_to_string(leaf.path().join("pids.max")).expect("read"),
            "4096"
        );
        assert!(
            !leaf.path().join("memory.max").exists(),
            "an unlimited unit gives the leaf no memory limit"
        );
        // A limit the unit sets that the leaf cannot take is not carried.
        fs::write(unit.path().join("memory.max"), "1073741824\n").expect("memory.max");
        assert!(!carry_unit_limits(unit.path(), &leaf.path().join("absent")));
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
