//! The answer to a prune: every entry, the verdict on it, and why.

use serde::Serialize;

/// Why a reference is pinned.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PinSource {
    /// The request's keep list.
    Keep,
    /// `SOMA_PREWARM_REFERENCE`, the shape prepared before traffic.
    PrewarmReference,
    /// A prewarm target set at runtime (ADR 0052).
    PrewarmTarget,
    /// The image the public runner launches.
    RunnerLaunch,
}

impl PinSource {
    /// The stable name a surface reports this source by.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::PrewarmReference => "prewarm_reference",
            Self::PrewarmTarget => "prewarm_target",
            Self::RunnerLaunch => "runner_launch",
        }
    }
}

/// One pinned reference and every reason it is pinned.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Pin {
    pub reference: String,
    pub sources: Vec<PinSource>,
}

/// Why an entry is kept, removed, or refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PruneReason {
    /// Kept: its reference is pinned.
    Pinned,
    /// Kept: the newest certified entry of its family older than the newest pinned one, or the
    /// newest of a family with no pinned entry.
    PreviousInFamily,
    /// Kept: the newest unpinned entry of its family, certified after its newest pinned one.
    NewerThanPinned,
    /// Kept: a Candidate not yet certified, which may still be being certified.
    Uncertified,
    /// Kept: the entry cannot be read well enough to classify.
    Unreadable,
    /// Kept: another entry claims the same reference.
    Ambiguous,
    /// Removed: certified, unpinned, and superseded within its family.
    Superseded,
    /// Removed: a previous run moved it out of the root and did not finish deleting it.
    Interrupted,
    /// Refused: an Instance's record names its Generation.
    HasInstances,
    /// Refused: the service holds its Generation somewhere other than its cache.
    InUse,
    /// Refused: an Instance record does not say which Generation it holds.
    UnattributedInstance,
    /// Refused: a head on disk belongs to no Instance whose Generation is known.
    UnattributedHead,
    /// Refused: it could not be moved out of the root.
    MoveFailed,
    /// Refused: it was moved out of the root but could not be deleted.
    DeleteFailed,
}

/// One entry in the plan.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PrunedEntry {
    /// The entry's directory name.
    pub entry: String,
    pub reference: Option<String>,
    /// The family the keep rule groups it in: the image and the tag up to its first `-`.
    pub family: Option<String>,
    pub generation_id: Option<String>,
    /// When `generation.id` was published, in seconds since the Unix epoch.
    pub certified_at: Option<u64>,
    pub reason: PruneReason,
    /// What the reason refers to: Instance ids, head paths, pin sources, or an error.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub detail: Vec<String>,
    /// Bytes in extents no other file shares; absent when they could not be measured.
    pub unshared_bytes: Option<u64>,
    /// Bytes in every mapped extent, shared or not.
    pub mapped_bytes: Option<u64>,
}

/// The answer to a prune: what is kept, removed, and refused, and why.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PrunePlan {
    pub root: String,
    /// Whether `remove` was acted on. False means nothing changed.
    pub applied: bool,
    pub pins: Vec<Pin>,
    /// Kept references that name no entry on this host.
    pub unmatched_keep: Vec<String>,
    pub keep: Vec<PrunedEntry>,
    pub remove: Vec<PrunedEntry>,
    pub refused: Vec<PrunedEntry>,
    /// The unshared bytes of `remove`.
    ///
    /// A lower bound on what removing them frees: extents two removed entries share are freed
    /// too, but neither entry counts them as its own.
    pub unshared_bytes: u64,
}
