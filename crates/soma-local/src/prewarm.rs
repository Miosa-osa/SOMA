//! Prewarm targets set at runtime, per shape (ADR 0052).
//!
//! A host keeps identity-free machines prepared so a launch skips restore and device
//! construction. The shapes and counts are a desired set that a caller replaces whole, reads back,
//! and that survives a restart. Nothing here starts, stops, or touches a sandbox that is running.

use serde::{Deserialize, Serialize};

/// Most targets one request may carry.
pub const MAX_TARGETS: usize = 16;
/// Most machines one target may ask for.
pub const MAX_TARGET_COUNT: usize = 512;

/// One shape to keep prepared: the image reference of a prepared Generation, and how many.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrewarmTarget {
    /// The reference a Generation was prepared for on this host.
    pub reference: String,
    /// How many machines to keep prepared for it; zero retires the shape's machines.
    pub count: usize,
    /// The boot memory of the Generation, when the caller wants it checked.
    ///
    /// Boot memory is a property of the prepared Generation, so this can only confirm it: a value
    /// that disagrees is refused rather than used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mib: Option<u64>,
}

/// What one target holds now.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TargetStatus {
    pub reference: String,
    pub generation_id: String,
    pub memory_mib: u64,
    /// How many were asked for.
    pub target: usize,
    /// How many are ready now.
    pub prepared: usize,
    /// Whether a machine for this target is being built.
    pub building: bool,
    /// Why the last build failed, if it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_failure: Option<String>,
    /// Launches for this shape that found a prepared machine.
    pub claimed_primed: u64,
    /// Launches for this shape that found none and built their own.
    pub claimed_miss: u64,
}

/// Launches for a shape no target covers, so a planner can see demand it has not yet planned for.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct UncoveredDemand {
    pub reference: String,
    pub memory_mib: u64,
    /// The memory the Generation boots with, which is the only memory a target can name for it.
    pub boot_memory_mib: u64,
    /// Whether a target could serve this demand: its memory is the Generation's boot memory.
    pub targetable: bool,
    pub claimed_primed: u64,
    pub claimed_miss: u64,
}

/// Where the desired set came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// A set a caller set, persisted under the state root.
    Persisted,
    /// The environment's single shape, as before this API existed.
    EnvDefault,
}

/// The desired set and what exists.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PrewarmStatus {
    pub source: Source,
    /// Set when a persisted set existed and could not be read; the environment defaults applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persisted_error: Option<String>,
    pub budget_mib: u64,
    /// Memory the desired set asks for: the sum of `count * memory_mib`.
    pub committed_mib: u64,
    pub unprimed_target: usize,
    pub unprimed_prepared: usize,
    pub targets: Vec<TargetStatus>,
    pub uncovered_demand: Vec<UncoveredDemand>,
}

/// Why a desired set was refused, or could not be applied.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PrewarmError {
    /// No prepared Generation matches this reference.
    UnknownReference(String),
    /// The set is malformed; the reason names the first problem.
    InvalidTarget(String),
    /// The set asks for more memory than the host budget.
    BudgetExceeded { budget_mib: u64, requested_mib: u64 },
    /// This target prepares nothing.
    Unsupported,
    /// The set was valid but could not be persisted.
    Unpersisted(String),
    /// The prepared store could not be read, or an entry in it is linked, ambiguous or damaged.
    StoreUnavailable(String),
}

impl PrewarmError {
    /// The stable name a surface reports this refusal by.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnknownReference(_) => "unknown_reference",
            Self::InvalidTarget(_) => "invalid_target",
            Self::BudgetExceeded { .. } => "prewarm_budget_exceeded",
            Self::Unsupported => "prewarm_unsupported",
            Self::Unpersisted(_) => "prewarm_not_persisted",
            Self::StoreUnavailable(_) => "prewarm_store_unavailable",
        }
    }
}

/// The persisted form of the desired set.
///
/// Behind the same gate as its only reader and writer,
/// `backend::kvm::targets::persist`, so the ARM64 client that has no KVM backend does not carry
/// an unused type.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) struct PersistedTargets {
    pub(crate) version: u32,
    pub(crate) targets: Vec<PrewarmTarget>,
}

/// File the desired set is persisted in, beside the rest of the state.
pub(crate) const PERSISTED_FILE: &str = "prewarm-targets.json";
