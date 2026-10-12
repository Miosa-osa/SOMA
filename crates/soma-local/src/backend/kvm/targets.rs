//! The prewarm targets a caller sets at runtime: validation, budget, persistence, readback.
//!
//! The pool itself lives in `host::sterile`; this module decides what a desired set is allowed
//! to be, keeps it across restarts, and reports it (ADR 0052).

use std::{
    io::Write as _,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use soma::BackendFailureKind;

use crate::prewarm::{
    MAX_TARGET_COUNT, MAX_TARGETS, PERSISTED_FILE, PersistedTargets, PrewarmError, PrewarmStatus,
    PrewarmTarget, Source, TargetStatus, UncoveredDemand,
};

use super::{host, prepared};

/// Budget for the memory the desired set may ask for, in MiB.
const BUDGET: &str = "SOMA_PREWARM_BUDGET_MIB";
/// Share of host memory the budget defaults to, in percent.
const DEFAULT_BUDGET_PERCENT: u64 = 60;

struct Manager {
    state_root: Option<PathBuf>,
    source: Source,
    persisted_error: Option<String>,
    committed_mib: u64,
}

fn manager() -> &'static Mutex<Manager> {
    static MANAGER: OnceLock<Mutex<Manager>> = OnceLock::new();
    MANAGER.get_or_init(|| {
        Mutex::new(Manager {
            state_root: None,
            source: Source::EnvDefault,
            persisted_error: None,
            committed_mib: 0,
        })
    })
}

/// Prepares the pool before the service accepts traffic.
///
/// A persisted set wins over the environment. With none, or with one that cannot be applied, the
/// environment's single shape applies exactly as it did before targets existed, and a persisted
/// set that could not be applied is reported instead of being half-applied.
pub(crate) fn start(state_root: &Path, workers: usize) -> Result<(), BackendFailureKind> {
    let mut guard = lock();
    guard.state_root = Some(state_root.to_path_buf());
    match load(state_root) {
        Ok(Some(set)) => match resolve(&set).and_then(|plans| {
            let committed = committed_mib(&plans)?;
            Ok((plans, committed))
        }) {
            Ok((plans, committed)) => {
                host::install_sets(plans, 0);
                guard.source = Source::Persisted;
                guard.committed_mib = committed;
                return Ok(());
            }
            Err(error) => guard.persisted_error = Some(describe(&error)),
        },
        Ok(None) => {}
        Err(reason) => guard.persisted_error = Some(reason),
    }
    drop(guard);
    super::prewarm_machine_hosts(workers)
}

/// Replaces the desired set, applying it whole or not at all.
pub(crate) fn set(targets: &[PrewarmTarget]) -> Result<PrewarmStatus, PrewarmError> {
    let plans = resolve(targets)?;
    let committed = committed_mib(&plans)?;
    let mut guard = lock();
    let Some(root) = guard.state_root.clone() else {
        return Err(PrewarmError::Unsupported);
    };
    persist(&root, targets).map_err(PrewarmError::Unpersisted)?;
    host::install_sets(plans, 0);
    guard.source = Source::Persisted;
    guard.persisted_error = None;
    guard.committed_mib = committed;
    drop(guard);
    Ok(status())
}

pub(crate) fn status() -> PrewarmStatus {
    let (source, persisted_error, committed_mib) = {
        let guard = lock();
        (
            guard.source,
            guard.persisted_error.clone(),
            guard.committed_mib,
        )
    };
    let pool = host::snapshot();
    PrewarmStatus {
        source,
        persisted_error,
        budget_mib: budget_mib(),
        committed_mib,
        unprimed_target: pool.generic_target,
        unprimed_prepared: pool.generic_prepared,
        targets: pool
            .targets
            .into_iter()
            .map(|target| TargetStatus {
                reference: target.reference,
                generation_id: target.generation_id,
                memory_mib: target.memory_mib,
                target: target.target,
                prepared: target.prepared,
                building: target.building,
                last_failure: target.last_failure,
                claimed_primed: target.claimed_primed,
                claimed_miss: target.claimed_miss,
            })
            .collect(),
        uncovered_demand: pool
            .uncovered_demand
            .into_iter()
            .map(|demand| UncoveredDemand {
                reference: demand.reference,
                memory_mib: demand.memory_mib,
                claimed_primed: demand.claimed_primed,
                claimed_miss: demand.claimed_miss,
            })
            .collect(),
    }
}

fn lock() -> std::sync::MutexGuard<'static, Manager> {
    manager()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn describe(error: &PrewarmError) -> String {
    match error {
        PrewarmError::UnknownReference(reference) => format!("unknown reference {reference}"),
        PrewarmError::InvalidTarget(reason) | PrewarmError::Unpersisted(reason) => reason.clone(),
        PrewarmError::BudgetExceeded {
            budget_mib,
            requested_mib,
        } => format!("{requested_mib} MiB is over the {budget_mib} MiB budget"),
        PrewarmError::Unsupported => "this host prepares nothing".to_owned(),
    }
}

/// Checks a set and resolves every reference to the prepared Generation it names.
fn resolve(targets: &[PrewarmTarget]) -> Result<Vec<host::TargetPlan>, PrewarmError> {
    resolve_in(prepared::store_root().as_deref(), targets)
}

fn resolve_in(
    root: Option<&Path>,
    targets: &[PrewarmTarget],
) -> Result<Vec<host::TargetPlan>, PrewarmError> {
    if targets.len() > MAX_TARGETS {
        return Err(PrewarmError::InvalidTarget(format!(
            "at most {MAX_TARGETS} targets"
        )));
    }
    // Every shape problem is named before any reference is looked up, so a malformed set is
    // refused for what is wrong with it and not for the first reference that happens to be absent.
    let mut seen = std::collections::BTreeSet::new();
    for target in targets {
        let reference = target.reference.trim();
        if reference.is_empty() {
            return Err(PrewarmError::InvalidTarget(
                "a reference is empty".to_owned(),
            ));
        }
        if target.count > MAX_TARGET_COUNT {
            return Err(PrewarmError::InvalidTarget(format!(
                "{reference}: count is at most {MAX_TARGET_COUNT}"
            )));
        }
        if !seen.insert(reference.to_owned()) {
            return Err(PrewarmError::InvalidTarget(format!(
                "{reference} appears twice"
            )));
        }
    }
    let root = root.ok_or(PrewarmError::Unsupported)?;
    let mut plans = Vec::with_capacity(targets.len());
    for target in targets {
        let reference = target.reference.trim();
        let found = prepared::find(Some(root), reference)
            .map_err(|_| PrewarmError::UnknownReference(reference.to_owned()))?;
        let memory_mib = found.manifest.shape.memory_bytes / (1024 * 1024);
        if let Some(asked) = target.memory_mib
            && asked != memory_mib
        {
            return Err(PrewarmError::InvalidTarget(format!(
                "{reference}: boot memory is {memory_mib} MiB, not {asked}"
            )));
        }
        plans.push(host::TargetPlan {
            plan: host::PrewarmPlan::new(found, memory_mib),
            count: target.count,
        });
    }
    Ok(plans)
}

fn committed_mib(plans: &[host::TargetPlan]) -> Result<u64, PrewarmError> {
    let asks = plans
        .iter()
        .map(|target| (target.count, target.plan.memory_mib()))
        .collect::<Vec<_>>();
    check_budget(&asks, budget_mib())
}

/// The memory a set asks for, or the refusal when it is over `budget_mib`.
fn check_budget(asks: &[(usize, u64)], budget_mib: u64) -> Result<u64, PrewarmError> {
    let requested_mib = asks.iter().fold(0_u64, |total, (count, memory_mib)| {
        total.saturating_add(
            u64::try_from(*count)
                .unwrap_or(u64::MAX)
                .saturating_mul(*memory_mib),
        )
    });
    if requested_mib > budget_mib {
        return Err(PrewarmError::BudgetExceeded {
            budget_mib,
            requested_mib,
        });
    }
    Ok(requested_mib)
}

/// The most memory the desired set may ask for.
fn budget_mib() -> u64 {
    if let Some(explicit) = std::env::var(BUDGET)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
    {
        return explicit;
    }
    host_memory_mib().map_or(0, |total| total * DEFAULT_BUDGET_PERCENT / 100)
}

fn host_memory_mib() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(kib / 1024)
}

/// Reads the persisted set. `Ok(None)` is no file; `Err` is a file that exists and cannot be used.
fn load(state_root: &Path) -> Result<Option<Vec<PrewarmTarget>>, String> {
    let path = state_root.join(PERSISTED_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("could not read {PERSISTED_FILE}: {}", error.kind())),
    };
    let persisted: PersistedTargets = serde_json::from_slice(&bytes)
        .map_err(|_| format!("{PERSISTED_FILE} is not a valid prewarm set"))?;
    if persisted.version != 1 {
        return Err(format!("{PERSISTED_FILE} has an unknown version"));
    }
    Ok(Some(persisted.targets))
}

/// Writes the set to a temporary file and renames it, so a reader sees the old set or the new one.
fn persist(state_root: &Path, targets: &[PrewarmTarget]) -> Result<(), String> {
    let body = serde_json::to_vec_pretty(&PersistedTargets {
        version: 1,
        targets: targets.to_vec(),
    })
    .map_err(|_| "the set could not be encoded".to_owned())?;
    let temporary = state_root.join(format!("{PERSISTED_FILE}.tmp"));
    let _ = std::fs::remove_file(&temporary);
    let result = (|| -> std::io::Result<()> {
        use std::os::unix::fs::OpenOptionsExt as _;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&body)?;
        file.sync_all()?;
        std::fs::rename(&temporary, state_root.join(PERSISTED_FILE))?;
        // The rename is durable once the directory is.
        std::fs::File::open(state_root)?.sync_all()
    })();
    result.map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        format!("could not persist the set: {}", error.kind())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(reference: &str, count: usize) -> PrewarmTarget {
        PrewarmTarget {
            reference: reference.to_owned(),
            count,
            memory_mib: None,
        }
    }

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "soma-targets-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).expect("create the test root");
        root
    }

    #[test]
    fn a_host_with_no_store_prepares_nothing_and_says_so() {
        assert_eq!(
            resolve_in(None, &[target("node:22", 1)]).err(),
            Some(PrewarmError::Unsupported)
        );
    }

    #[test]
    fn a_reference_with_no_prepared_generation_is_named() {
        let root = scratch("unknown");
        assert_eq!(
            resolve_in(Some(&root), &[target("node:22", 2)]).err(),
            Some(PrewarmError::UnknownReference("node:22".to_owned()))
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_malformed_set_is_refused_before_any_reference_is_looked_up() {
        let root = scratch("malformed");
        for (set, reason) in [
            (vec![target("", 1)], "a reference is empty"),
            (vec![target("   ", 1)], "a reference is empty"),
            (vec![target("a", MAX_TARGET_COUNT + 1)], "count is at most"),
            (vec![target("a", 1), target("a", 2)], "appears twice"),
            (
                (0..=MAX_TARGETS)
                    .map(|n| target(&format!("r{n}"), 1))
                    .collect(),
                "at most",
            ),
        ] {
            match resolve_in(Some(&root), &set) {
                Err(PrewarmError::InvalidTarget(text)) => {
                    assert!(text.contains(reason), "{text} should mention {reason}");
                }
                other => panic!("expected an invalid target, got {:?}", other.err()),
            }
        }
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn the_budget_counts_count_times_memory_and_refuses_over_it() {
        assert_eq!(check_budget(&[(10, 4096), (5, 8192)], 100_000), Ok(81_920));
        assert_eq!(check_budget(&[], 0), Ok(0));
        assert_eq!(
            check_budget(&[(10, 4096), (5, 8192)], 81_919),
            Err(PrewarmError::BudgetExceeded {
                budget_mib: 81_919,
                requested_mib: 81_920,
            })
        );
        // A count that would overflow is still a refusal, never a wrap to a small number.
        assert!(check_budget(&[(usize::MAX, u64::MAX)], u64::MAX - 1).is_err());
    }

    #[test]
    fn a_persisted_set_round_trips_and_a_missing_file_is_no_set() {
        let root = scratch("persist");
        assert_eq!(load(&root), Ok(None));
        let set = vec![target("node:22", 20), target("python:3.12", 4)];
        persist(&root, &set).expect("persist");
        assert_eq!(load(&root), Ok(Some(set)));
        // The temporary file does not outlive a successful write.
        assert!(!root.join(format!("{PERSISTED_FILE}.tmp")).exists());
        // A second write replaces the first whole.
        persist(&root, &[]).expect("replace");
        assert_eq!(load(&root), Ok(Some(Vec::new())));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_persisted_file_is_private_and_a_damaged_one_is_reported_not_used() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = scratch("damaged");
        persist(&root, &[target("node:22", 1)]).expect("persist");
        let mode = std::fs::metadata(root.join(PERSISTED_FILE))
            .expect("stat")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);

        for bad in [
            &b"not json"[..],
            br#"{"version":2,"targets":[]}"#,
            br#"{"version":1,"targets":[],"extra":1}"#,
            br#"{"version":1,"targets":[{"reference":"a","count":1,"surprise":true}]}"#,
        ] {
            std::fs::write(root.join(PERSISTED_FILE), bad).expect("write");
            assert!(load(&root).is_err(), "{}", String::from_utf8_lossy(bad));
        }
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_failed_write_leaves_the_previous_set_in_place() {
        let root = scratch("failed-write");
        persist(&root, &[target("node:22", 3)]).expect("persist");
        // A directory squatting on the temporary name makes the write fail.
        let squat = root.join(format!("{PERSISTED_FILE}.tmp"));
        std::fs::create_dir(&squat).expect("squat");
        // The temporary file is removed first, so this write succeeds; make it fail by pointing
        // at a root that does not exist instead.
        assert!(persist(&root.join("absent"), &[]).is_err());
        assert_eq!(load(&root), Ok(Some(vec![target("node:22", 3)])));
        std::fs::remove_dir_all(root).ok();
    }
}
