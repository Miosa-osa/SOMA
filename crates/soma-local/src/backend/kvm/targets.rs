//! The prewarm targets a caller sets at runtime: validation, budget, persistence, readback.
//!
//! The pool itself lives in `host::sterile`; this module decides what a desired set is allowed
//! to be, keeps it across restarts, and reports it (ADR 0052).

mod persist;
mod resolve;
#[cfg(test)]
mod tests;

use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use soma::BackendFailureKind;

use crate::prewarm::{
    PrewarmError, PrewarmStatus, PrewarmTarget, Source, TargetStatus, UncoveredDemand,
};

use super::host;
use persist::{load, persist};
use resolve::{budget_mib, committed_mib, resolve};

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

/// Applies the desired set at start.
///
/// A persisted set wins over the environment. It is built in the background after start, one
/// machine at a time, so launches in the first moments can miss until the refill catches up (the
/// readback shows the progress); only the environment's single shape is prepared before the
/// service accepts traffic, exactly as it was before targets existed. With no persisted set, or
/// one that cannot be applied, the environment applies, and a persisted set that could not be
/// applied is reported instead of being half-applied.
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
                boot_memory_mib: demand.boot_memory_mib,
                targetable: demand.memory_mib == demand.boot_memory_mib,
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
        PrewarmError::InvalidTarget(reason)
        | PrewarmError::Unpersisted(reason)
        | PrewarmError::StoreUnavailable(reason) => reason.clone(),
        PrewarmError::BudgetExceeded {
            budget_mib,
            requested_mib,
        } => format!("{requested_mib} MiB is over the {budget_mib} MiB budget"),
        PrewarmError::Unsupported => "this host prepares nothing".to_owned(),
    }
}
