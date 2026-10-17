//! Who owns a sandbox on this runner, and why a call on one cannot start.

use std::{
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::{Duration, Instant},
};

/// Who a sandbox belongs to, as the runner recorded it at create.
#[derive(Clone, Debug)]
pub struct Owner {
    pub tenant_id: String,
    /// The creating key; absent for a sandbox recovered from the state store after a restart.
    pub key_id: Option<String>,
    pub project_id: Option<String>,
    pub created: Instant,
    /// The tenant's live sandbox counter; the sandbox gives its slot back when it ends.
    pub slot: Arc<AtomicI64>,
}

impl PartialEq for Owner {
    fn eq(&self, other: &Self) -> bool {
        self.tenant_id == other.tenant_id
            && self.key_id == other.key_id
            && self.project_id == other.project_id
            && self.created == other.created
    }
}

impl Eq for Owner {}

impl Owner {
    pub(crate) fn give_back_slot(&self) {
        self.slot.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Why a lifecycle call on an existing sandbox cannot start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// Not here, not this tenant's, or still being created: all answer 404 alike, so a caller
    /// learns nothing about sandboxes it does not own.
    NotFound,
    Busy,
    /// Already destroyed: the owner, and how long the sandbox lived.
    Destroyed(Owner, Duration),
    /// The launch this sandbox was answered for failed, so the sandbox was never created.
    LaunchFailed(LaunchFailure),
}

/// A launch that failed after its create had already answered.
///
/// A synchronous create reports its own failure; a create that answered early has already sent
/// its `201`, so the launch's failure is filed against the sandbox and the next call about it
/// reads the answer the synchronous create would have given. The two cases are kept apart
/// because they are a different status: a launch the engine refused is `503`, and a launch that
/// could not take a facade lease is the `429` every other call at this runner's admission cap
/// gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchFailure {
    /// The engine refused the launch, or it could not be completed. `code` is the engine's own
    /// stable one, for the operator's log rather than for the caller: the caller reads the
    /// create's `503`, exactly as it would have from a synchronous create.
    Unavailable { code: &'static str },
    /// No facade lease could be taken.
    Busy,
}
