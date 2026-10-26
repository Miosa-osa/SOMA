//! Releasing prepared Generations a host no longer needs (ADR 0054).
//!
//! A host keeps every Generation it ever prepared, and the running service holds open handles into
//! each one, so an operator cannot remove an entry safely by hand. A prune plans which entries are
//! still wanted, refuses any entry something still uses, and, only when asked to act, releases the
//! service's handles and removes the rest. Everything here is the shape of that request and its
//! answer; the work itself is the KVM backend's.

use std::{
    collections::BTreeSet,
    sync::{Mutex, OnceLock},
};

use serde::Deserialize;
use soma::{BackendKind, GenerationId, InstanceId, MachineListing};

mod plan;

pub use plan::{Pin, PinSource, PrunePlan, PruneReason, PrunedEntry};

/// Most references one request may name.
pub const MAX_KEEP: usize = 1024;

/// One prune request.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PruneRequest {
    /// The references the caller still needs, kept whatever their age.
    ///
    /// A control plane passes every reference it may launch here. A prewarm floor's
    /// `reference@memory_mib` key is accepted as it is: a trailing `@` and digits is a memory
    /// size, not part of the reference, so it is dropped. A digest (`@sha256:...`) is kept.
    #[serde(default)]
    pub keep: Vec<String>,
    /// Remove what the plan names. Without it the answer is the plan and nothing changes.
    #[serde(default)]
    pub apply: bool,
}

impl PruneRequest {
    /// Refuses a request that could remove what its caller still needs.
    ///
    /// An applied prune with no keep list is refused: the host's own pins name only what it
    /// prewarms, not what a control plane may launch next, so an empty list would leave every
    /// other Generation's survival to the family rule alone. A dry run with no keep list is
    /// allowed, because it changes nothing and shows exactly what the host's pins protect.
    ///
    /// # Errors
    ///
    /// Returns [`PruneError::KeepRequired`] or [`PruneError::InvalidKeep`].
    pub fn validate(&self) -> Result<(), PruneError> {
        if self.keep.len() > MAX_KEEP {
            return Err(PruneError::InvalidKeep(format!(
                "at most {MAX_KEEP} references may be kept"
            )));
        }
        if let Some(bad) = self
            .keep
            .iter()
            .find(|reference| soma::OciImage::parse(kept_reference(reference)).is_err())
        {
            return Err(PruneError::InvalidKeep(format!(
                "{bad:?} is not an image reference"
            )));
        }
        if self.apply && self.keep.is_empty() {
            return Err(PruneError::KeepRequired);
        }
        Ok(())
    }

    /// The references this request keeps, with any memory suffix dropped.
    #[must_use]
    pub fn kept(&self) -> BTreeSet<String> {
        self.keep
            .iter()
            .map(|reference| kept_reference(reference).to_owned())
            .collect()
    }
}

/// One kept reference without the `@<memory_mib>` a prewarm floor key carries.
fn kept_reference(reference: &str) -> &str {
    let reference = reference.trim();
    match reference.rsplit_once('@') {
        Some((image, memory))
            if !memory.is_empty() && memory.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            image
        }
        _ => reference,
    }
}

/// What the durable state says still holds a Generation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InstanceHolds {
    /// Every Instance whose record names the Generation it launched from.
    pub held: Vec<(InstanceId, GenerationId)>,
    /// Every Instance that may hold a Generation but whose record does not say which.
    ///
    /// A KVM record that names no Generation, and a record the store could not decode, could be
    /// holding any entry, so while one exists nothing is removed.
    pub unattributed: Vec<InstanceId>,
}

impl InstanceHolds {
    /// Reads what holds a Generation from one listing of the durable store.
    ///
    /// Only KVM records launch from the prepared store; a record another backend wrote holds
    /// nothing in it.
    #[must_use]
    pub fn from_listing(listing: &MachineListing) -> Self {
        let mut holds = Self::default();
        for entry in listing.entries() {
            match entry.generation_id() {
                Some(generation) => holds
                    .held
                    .push((entry.instance_id().clone(), generation.clone())),
                None if entry.backend() == BackendKind::LinuxKvm => {
                    holds.unattributed.push(entry.instance_id().clone());
                }
                None => {}
            }
        }
        holds.unattributed.extend(
            listing
                .unreadable()
                .iter()
                .map(|record| record.instance_id().clone()),
        );
        holds
    }
}

/// Why a prune could not be planned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PruneError {
    /// This target has no prepared store.
    Unsupported,
    /// The prepared root is unset or cannot be read.
    StoreUnavailable(String),
    /// An applied prune named no reference to keep.
    KeepRequired,
    /// A kept reference is not one.
    InvalidKeep(String),
    /// What holds a Generation could not be read, so nothing can be shown unused.
    InstancesUnavailable(String),
    /// Another prune is running.
    Busy,
}

impl PruneError {
    /// The stable name a surface reports this refusal by.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Unsupported => "prune_unsupported",
            Self::StoreUnavailable(_) => "prune_store_unavailable",
            Self::KeepRequired => "keep_required",
            Self::InvalidKeep(_) => "invalid_keep",
            Self::InstancesUnavailable(_) => "instances_unavailable",
            Self::Busy => "prune_in_progress",
        }
    }
}

/// Pins a reference this process launches from its own configuration.
///
/// The public runner launches one configured image, and a prune must never remove the entry it
/// launches from even when a caller's keep list forgets it.
pub fn pin_launch_reference(reference: &str) {
    let reference = reference.trim();
    if !reference.is_empty() {
        launch_pins()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(reference.to_owned());
    }
}

/// The references this process pinned from its configuration.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) fn launch_references() -> Vec<String> {
    launch_pins()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .cloned()
        .collect()
}

fn launch_pins() -> &'static Mutex<BTreeSet<String>> {
    static PINS: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    PINS.get_or_init(|| Mutex::new(BTreeSet::new()))
}

#[cfg(test)]
#[path = "prune_tests.rs"]
mod tests;
