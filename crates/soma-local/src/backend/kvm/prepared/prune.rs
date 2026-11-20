//! Releasing prepared Generations this host no longer needs (ADR 0054).
//!
//! A prune reads the root, decides every entry by the keep rule (`policy`), and refuses any entry
//! an Instance or an unattributed head may still hold. Only an applied prune changes anything, and
//! it releases this process's handles before it removes a byte (`remove`). Everything it decided
//! is in the answer, so a dry run is exactly the applied run's plan.

mod heads;
mod policy;
mod remove;
mod survey;
#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Mutex, TryLockError},
    time::UNIX_EPOCH,
};

use soma::GenerationId;

use super::{
    CacheKey, STORE,
    cache::{Cache, Retained, generations},
    scan::store_root,
};
use crate::{
    InstanceHolds, Pin, PinSource, PruneError, PrunePlan, PruneReason, PruneRequest, PrunedEntry,
};
use policy::{Decision, Holders, Verdict};
use survey::{State, Surveyed};

/// Reads what the durable store says holds a Generation.
type Holdings<'a> = &'a mut dyn FnMut() -> Result<InstanceHolds, String>;

/// What one prune runs on.
pub(super) struct Inputs {
    pub(super) root: PathBuf,
    pub(super) head_root: PathBuf,
    pub(super) pins: BTreeMap<String, BTreeSet<PinSource>>,
    pub(super) apply: bool,
    pub(super) keep_previous: bool,
}

/// Plans, and when asked applies, a prune of this host's prepared root.
///
/// One prune runs at a time: two applied at once could each move an entry the other is
/// re-checking, and neither would see the other's second look.
pub(crate) fn prune(
    request: &PruneRequest,
    instances: Holdings<'_>,
) -> Result<PrunePlan, PruneError> {
    static RUNNING: Mutex<()> = Mutex::new(());
    let _running = match RUNNING.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return Err(PruneError::Busy),
    };
    let root =
        store_root().ok_or_else(|| PruneError::StoreUnavailable(format!("{STORE} is unset")))?;
    let inputs = Inputs {
        root,
        head_root: crate::backend::kvm::boot::head_root(),
        pins: pins(request),
        apply: request.apply,
        keep_previous: request.keep_previous,
    };
    run(&inputs, generations(), instances)
}

/// Every pinned reference, with every reason it is pinned.
fn pins(request: &PruneRequest) -> BTreeMap<String, BTreeSet<PinSource>> {
    let mut pins: BTreeMap<String, BTreeSet<PinSource>> = BTreeMap::new();
    let mut pin = |reference: String, source| {
        pins.entry(reference).or_default().insert(source);
    };
    for reference in request.kept() {
        pin(reference, PinSource::Keep);
    }
    if let Ok(reference) = std::env::var(crate::backend::kvm::PREWARM_REFERENCE)
        && !reference.trim().is_empty()
    {
        pin(reference.trim().to_owned(), PinSource::PrewarmReference);
    }
    for target in crate::backend::kvm::targets::status().targets {
        pin(target.reference, PinSource::PrewarmTarget);
    }
    for reference in crate::prune::launch_references() {
        pin(reference, PinSource::RunnerLaunch);
    }
    pins
}

/// Plans the prune of `inputs.root` against `cache`, and applies it when asked.
pub(super) fn run<T: Clone + Retained>(
    inputs: &Inputs,
    cache: &Cache<T>,
    instances: Holdings<'_>,
) -> Result<PrunePlan, PruneError> {
    let (entries, leftovers) = survey::survey(&inputs.root)?;
    let holds = instances().map_err(PruneError::InstancesUnavailable)?;
    let holders = Holders::new(&holds, heads::named_heads(&inputs.head_root));
    let mut decisions = policy::decide_with(&entries, &inputs.pins, &holders, inputs.keep_previous);
    // The dry run says what the applied run would find: a value held outside the cache is
    // refused here too, rather than only when a release is attempted.
    for (entry, decision) in entries.iter().zip(decisions.iter_mut()) {
        if decision.verdict == Verdict::Remove && cache.held_elsewhere(&key(&inputs.root, entry)) {
            *decision = remove::held_elsewhere();
        }
    }
    // A leftover was already chosen for removal by the run that moved it; only a holder stops it.
    let mut finished: Vec<Decision> = leftovers
        .iter()
        .map(|leftover| {
            holders
                .refusal(generation(leftover))
                .unwrap_or_else(|| Decision::remove(PruneReason::Interrupted))
        })
        .collect();
    if inputs.apply {
        apply(inputs, cache, &entries, &mut decisions, instances);
        for (leftover, decision) in leftovers.iter().zip(finished.iter_mut()) {
            if decision.verdict == Verdict::Remove
                && let Err(refusal) = remove::delete(&leftover.path)
            {
                *decision = refusal;
            }
        }
    }
    Ok(report(inputs, &entries, decisions, &leftovers, finished))
}

/// Takes every entry the plan removes out of service, looks at the holders again, and deletes
/// each entry nothing claimed in between.
///
/// A launch that found an entry before it left the cache recorded itself before it used it, so
/// the second look sees that launch and its entry is put back whole.
fn apply<T: Clone + Retained>(
    inputs: &Inputs,
    cache: &Cache<T>,
    entries: &[Surveyed],
    decisions: &mut [Decision],
    instances: Holdings<'_>,
) {
    let chosen: Vec<usize> = (0..entries.len())
        .filter(|index| decisions[*index].verdict == Verdict::Remove)
        .collect();
    if chosen.is_empty() {
        return;
    }
    let holding = match remove::holding_directory(&inputs.root) {
        Ok(holding) => holding,
        Err(reason) => {
            for index in chosen {
                decisions[index] = Decision::refuse(PruneReason::MoveFailed, vec![reason.clone()]);
            }
            return;
        }
    };
    let mut moved = Vec::new();
    for index in chosen {
        let entry = &entries[index];
        match remove::take_out(cache, key(&inputs.root, entry), &entry.path, &holding) {
            Ok(out) => moved.push((index, out)),
            Err(refusal) => decisions[index] = refusal,
        }
    }
    if moved.is_empty() {
        return;
    }
    let again = instances().map(|holds| Holders::new(&holds, Ok(Vec::new())));
    for (index, out) in moved {
        let refusal = match &again {
            Ok(holders) => holders.refusal(generation(&entries[index])),
            Err(reason) => Some(Decision::refuse(
                PruneReason::UnattributedInstance,
                vec![format!("the holders could not be read again: {reason}")],
            )),
        };
        match refusal {
            Some(refusal) => decisions[index] = remove::put_back(cache, out, refusal),
            None => {
                if let Err(refusal) = remove::finish(cache, out) {
                    decisions[index] = refusal;
                }
            }
        }
    }
}

fn key(root: &Path, entry: &Surveyed) -> CacheKey {
    (
        root.to_path_buf(),
        entry.reference.clone().unwrap_or_default(),
    )
}

const fn generation(entry: &Surveyed) -> Option<&GenerationId> {
    match &entry.state {
        State::Certified { id, .. } => Some(id),
        State::Uncertified | State::Unreadable(_) => None,
    }
}

fn report(
    inputs: &Inputs,
    entries: &[Surveyed],
    decisions: Vec<Decision>,
    leftovers: &[Surveyed],
    finished: Vec<Decision>,
) -> PrunePlan {
    let present: BTreeSet<&str> = entries
        .iter()
        .filter_map(|entry| entry.reference.as_deref())
        .collect();
    let mut plan = PrunePlan {
        root: inputs.root.display().to_string(),
        applied: inputs.apply,
        pins: inputs
            .pins
            .iter()
            .map(|(reference, sources)| Pin {
                reference: reference.clone(),
                sources: sources.iter().copied().collect(),
            })
            .collect(),
        unmatched_keep: inputs
            .pins
            .iter()
            .filter(|(reference, sources)| {
                sources.contains(&PinSource::Keep) && !present.contains(reference.as_str())
            })
            .map(|(reference, _)| reference.clone())
            .collect(),
        keep: Vec::new(),
        remove: Vec::new(),
        refused: Vec::new(),
        unshared_bytes: 0,
    };
    let decided = entries.iter().zip(decisions);
    for (entry, decision) in decided.chain(leftovers.iter().zip(finished)) {
        let verdict = decision.verdict;
        let pruned = pruned(entry, decision);
        match verdict {
            Verdict::Keep => plan.keep.push(pruned),
            Verdict::Remove => {
                plan.unshared_bytes = plan
                    .unshared_bytes
                    .saturating_add(pruned.unshared_bytes.unwrap_or_default());
                plan.remove.push(pruned);
            }
            Verdict::Refuse => plan.refused.push(pruned),
        }
    }
    plan
}

fn pruned(entry: &Surveyed, decision: Decision) -> PrunedEntry {
    let certified_at = match &entry.state {
        State::Certified { at, .. } => at.duration_since(UNIX_EPOCH).ok().map(|at| at.as_secs()),
        State::Uncertified | State::Unreadable(_) => None,
    };
    PrunedEntry {
        entry: entry.name.clone(),
        reference: entry.reference.clone(),
        family: entry.reference.as_deref().map(policy::family),
        generation_id: generation(entry).map(|id| id.as_str().to_owned()),
        certified_at,
        reason: decision.reason,
        detail: decision.detail,
        unshared_bytes: entry.bytes.map(|bytes| bytes.unshared),
        mapped_bytes: entry.bytes.map(|bytes| bytes.mapped),
    }
}
