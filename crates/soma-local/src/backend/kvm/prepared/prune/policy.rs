//! Which entries a host keeps, which it may remove, and what refuses a removal.
//!
//! The rule, decided by the SOMA owner (ADR 0054):
//!
//! 1. Keep every pinned entry.
//! 2. Keep the newest previous certified entry of each reference family, unless the request asks
//!    to drop it (`keep_previous: false`) and its keep list names a pin of that family.
//! 3. Refuse to remove anything an Instance or a head still holds.
//!
//! Where the rule is ambiguous it keeps more, not less: an entry that is not certified, cannot be
//! read, or shares its reference with another entry is kept, and a family whose newest unpinned
//! entry is newer than its newest pinned one keeps that entry and the previous one both.

use std::collections::{BTreeMap, BTreeSet};

use soma::GenerationId;

use super::{
    heads::NamedHead,
    survey::{State, Surveyed},
};
use crate::{InstanceHolds, PinSource, PruneReason};

/// What one entry is to become.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Verdict {
    Keep,
    Remove,
    Refuse,
}

/// The verdict on one entry, why, and what the reason refers to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Decision {
    pub(super) verdict: Verdict,
    pub(super) reason: PruneReason,
    pub(super) detail: Vec<String>,
}

impl Decision {
    pub(super) const fn keep(reason: PruneReason, detail: Vec<String>) -> Self {
        Self {
            verdict: Verdict::Keep,
            reason,
            detail,
        }
    }

    pub(super) const fn remove(reason: PruneReason) -> Self {
        Self {
            verdict: Verdict::Remove,
            reason,
            detail: Vec::new(),
        }
    }

    pub(super) const fn refuse(reason: PruneReason, detail: Vec<String>) -> Self {
        Self {
            verdict: Verdict::Refuse,
            reason,
            detail,
        }
    }
}

/// Everything that holds a Generation, by Generation, and everything that cannot be attributed.
pub(super) struct Holders {
    instances: BTreeMap<GenerationId, Vec<String>>,
    unattributed_instances: Vec<String>,
    unattributed_heads: Vec<String>,
}

impl Holders {
    /// Ties each Instance to the Generation it holds, and each named head to its Instance.
    ///
    /// A head is tied through its name to the Instance it was made for. A head of a live Instance
    /// adds nothing, because that Instance's record already refuses its Generation. A head whose
    /// name is no live Instance outlived the Instance it was made for and shares extents with an
    /// entry nothing can name, so it is unattributed; so is a head root that could not be read.
    pub(super) fn new(holds: &InstanceHolds, heads: Result<Vec<NamedHead>, String>) -> Self {
        let mut instances: BTreeMap<GenerationId, Vec<String>> = BTreeMap::new();
        for (instance, generation) in &holds.held {
            instances
                .entry(generation.clone())
                .or_default()
                .push(instance.as_str().to_owned());
        }
        let mut holders = Self {
            instances,
            unattributed_instances: holds
                .unattributed
                .iter()
                .map(|instance| instance.as_str().to_owned())
                .collect(),
            unattributed_heads: Vec::new(),
        };
        let live = holds
            .held
            .iter()
            .map(|(instance, _)| instance)
            .chain(&holds.unattributed);
        let live: BTreeSet<String> = live
            .map(|instance| instance.as_str().to_ascii_lowercase())
            .collect();
        match heads {
            Ok(heads) => holders.unattributed_heads.extend(
                heads
                    .into_iter()
                    .filter(|head| !live.contains(&head.name.to_ascii_lowercase()))
                    .map(|head| head.path.display().to_string()),
            ),
            Err(reason) => holders.unattributed_heads.push(reason),
        }
        holders
    }

    /// Why nothing may remove `generation`, if anything says so.
    pub(super) fn refusal(&self, generation: Option<&GenerationId>) -> Option<Decision> {
        if let Some(instances) = generation.and_then(|generation| self.instances.get(generation)) {
            return Some(Decision::refuse(
                PruneReason::HasInstances,
                instances.clone(),
            ));
        }
        if !self.unattributed_instances.is_empty() {
            let detail = self.unattributed_instances.clone();
            return Some(Decision::refuse(PruneReason::UnattributedInstance, detail));
        }
        if !self.unattributed_heads.is_empty() {
            let detail = self.unattributed_heads.clone();
            return Some(Decision::refuse(PruneReason::UnattributedHead, detail));
        }
        None
    }
}

/// The family a reference belongs to: its image and the tag up to the tag's first `-`.
///
/// Versions of one image are cut as a tag suffix after the first `-`
/// (`capture-sized-generation.sh` builds `soma-default:${NAME}-${SUFFIX}`), so `soma-default:G10`,
/// `soma-default:G10-b2`, and `soma-default:G10-sr1` are one family and `soma-default:G20` is
/// another. A reference with no tag, a tag with no `-`, or a digest is its own family, which keeps
/// more rather than grouping references the rule cannot show are versions of one another.
pub(super) fn family(reference: &str) -> String {
    if reference.contains('@') {
        return reference.to_owned();
    }
    let name_start = reference.rfind('/').map_or(0, |slash| slash + 1);
    let Some(colon) = reference[name_start..].rfind(':') else {
        return reference.to_owned();
    };
    let (image, tag) = reference.split_at(name_start + colon);
    match tag[1..].split_once('-') {
        Some((prefix, _)) if !prefix.is_empty() => format!("{image}:{prefix}"),
        _ => reference.to_owned(),
    }
}

/// Decides every entry, in the order given, keeping every family's rollback target.
#[cfg(test)]
pub(super) fn decide(
    entries: &[Surveyed],
    pins: &BTreeMap<String, BTreeSet<PinSource>>,
    holders: &Holders,
) -> Vec<Decision> {
    decide_with(entries, pins, holders, true)
}

/// Decides every entry, in the order given.
///
/// With `keep_previous` false, a family whose pin the request's keep list names loses its
/// rollback target; a family pinned only by the host's own configuration, or not at all, keeps it.
pub(super) fn decide_with(
    entries: &[Surveyed],
    pins: &BTreeMap<String, BTreeSet<PinSource>>,
    holders: &Holders,
    keep_previous: bool,
) -> Vec<Decision> {
    let mut claims: BTreeMap<&str, usize> = BTreeMap::new();
    for reference in entries
        .iter()
        .filter_map(|entry| entry.reference.as_deref())
    {
        *claims.entry(reference).or_default() += 1;
    }
    let mut decisions = Vec::with_capacity(entries.len());
    // Certified entries the rule must order, by family: (index, published at, pinned).
    let mut families: BTreeMap<String, Vec<(usize, std::time::SystemTime, bool)>> = BTreeMap::new();
    for (index, entry) in entries.iter().enumerate() {
        let reference = entry.reference.as_deref().unwrap_or_default();
        let decision = match &entry.state {
            State::Unreadable(reason) => {
                Decision::keep(PruneReason::Unreadable, vec![reason.clone()])
            }
            _ if claims.get(reference).copied().unwrap_or_default() > 1 => {
                Decision::keep(PruneReason::Ambiguous, Vec::new())
            }
            State::Uncertified => Decision::keep(PruneReason::Uncertified, Vec::new()),
            State::Certified { at, .. } => {
                let sources = pins.get(reference);
                families.entry(family(reference)).or_default().push((
                    index,
                    *at,
                    sources.is_some(),
                ));
                match sources {
                    Some(sources) => Decision::keep(
                        PruneReason::Pinned,
                        sources
                            .iter()
                            .map(|source| source.code().to_owned())
                            .collect(),
                    ),
                    // Settled below, once the whole family has been seen.
                    None => Decision::remove(PruneReason::Superseded),
                }
            }
        };
        decisions.push(decision);
    }
    for members in families.values_mut() {
        let keep_rollback = keep_previous || !named_in_keep(entries, members, pins);
        settle_family(entries, members, &mut decisions, holders, keep_rollback);
    }
    decisions
}

/// Whether the request's keep list names a pinned entry of this family.
fn named_in_keep(
    entries: &[Surveyed],
    members: &[(usize, std::time::SystemTime, bool)],
    pins: &BTreeMap<String, BTreeSet<PinSource>>,
) -> bool {
    members.iter().any(|&(index, _, pinned)| {
        pinned
            && entries[index]
                .reference
                .as_deref()
                .and_then(|reference| pins.get(reference))
                .is_some_and(|sources| sources.contains(&PinSource::Keep))
    })
}

/// Keeps the newest unpinned entry of one family, and the previous one when that is newer than
/// every pinned entry; everything older is removed unless something holds it. Without
/// `keep_rollback` the previous one is not kept either.
fn settle_family(
    entries: &[Surveyed],
    members: &mut [(usize, std::time::SystemTime, bool)],
    decisions: &mut [Decision],
    holders: &Holders,
    keep_rollback: bool,
) {
    // Newest first; equal times fall back to the reference so the order never depends on the
    // filesystem's.
    members.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| entries[left.0].reference.cmp(&entries[right.0].reference))
    });
    let newest_pinned = members
        .iter()
        .find(|member| member.2)
        .map(|member| member.1);
    let mut newer_kept = false;
    let mut previous_kept = false;
    for &(index, at, pinned) in members.iter() {
        if pinned {
            continue;
        }
        let newer_than_pinned = newest_pinned.is_some_and(|pinned_at| at > pinned_at);
        decisions[index] = if newer_than_pinned && !newer_kept {
            newer_kept = true;
            Decision::keep(PruneReason::NewerThanPinned, Vec::new())
        } else if !newer_than_pinned && !previous_kept && keep_rollback {
            previous_kept = true;
            Decision::keep(PruneReason::PreviousInFamily, Vec::new())
        } else {
            let generation = match &entries[index].state {
                State::Certified { id, .. } => Some(id),
                _ => None,
            };
            holders
                .refusal(generation)
                .unwrap_or_else(|| Decision::remove(PruneReason::Superseded))
        };
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
