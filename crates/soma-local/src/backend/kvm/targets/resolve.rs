//! Checking a desired set: shape, store lookup, and the memory budget.

use std::path::Path;

use crate::prewarm::{MAX_TARGET_COUNT, MAX_TARGETS, PrewarmError, PrewarmTarget};

use crate::backend::kvm::{host, prepared};

/// Budget for the memory the desired set may ask for, in MiB.
const BUDGET: &str = "SOMA_PREWARM_BUDGET_MIB";
/// Share of host memory the budget defaults to, in percent.
const DEFAULT_BUDGET_PERCENT: u64 = 60;

/// Checks a set and resolves every reference to the prepared Generation it names.
pub(super) fn resolve(targets: &[PrewarmTarget]) -> Result<Vec<host::TargetPlan>, PrewarmError> {
    resolve_in(prepared::store_root().as_deref(), targets)
}

pub(super) fn resolve_in(
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
            .map_err(|error| store_error(&error, reference))?;
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

pub(super) fn committed_mib(plans: &[host::TargetPlan]) -> Result<u64, PrewarmError> {
    let asks = plans
        .iter()
        .map(|target| (target.count, target.plan.memory_mib()))
        .collect::<Vec<_>>();
    check_budget(&asks, budget_mib())
}

/// The memory a set asks for, or the refusal when it is over `budget_mib`.
pub(super) fn check_budget(asks: &[(usize, u64)], budget_mib: u64) -> Result<u64, PrewarmError> {
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
pub(super) fn budget_mib() -> u64 {
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

/// What a failed lookup means to the caller: an absent Generation is the caller's to correct,
/// while a store that cannot be read, a link, an ambiguous or a damaged entry is the host's.
fn store_error(error: &prepared::PreparedError, reference: &str) -> PrewarmError {
    use prepared::PreparedError as E;
    match error {
        E::NotPrepared | E::Uncertified => PrewarmError::UnknownReference(reference.to_owned()),
        E::StoreUnset => PrewarmError::Unsupported,
        E::StoreUnreadable => {
            PrewarmError::StoreUnavailable("the prepared store cannot be read".to_owned())
        }
        E::Linked => PrewarmError::StoreUnavailable(format!(
            "the prepared entry for {reference} is a symbolic link"
        )),
        E::Ambiguous => PrewarmError::StoreUnavailable(format!(
            "more than one prepared entry claims {reference}"
        )),
        E::Damaged => {
            PrewarmError::StoreUnavailable(format!("the prepared entry for {reference} is damaged"))
        }
    }
}
