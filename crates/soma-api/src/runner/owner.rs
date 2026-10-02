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
}
