use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};

/// One API key the control plane has published to this runner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyRecord {
    pub key_id: String,
    pub tenant_id: String,
    pub user_id: Option<String>,
    pub projects: Projects,
    pub rate_per_second: Option<u32>,
}

/// The projects a key may create sandboxes in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Projects {
    All,
    Only(HashSet<String>),
}

impl Projects {
    /// Whether a create naming `project` is inside this key's scope.
    ///
    /// A create that names no project is allowed, as it is on the standard path, where
    /// `authorize_project/2` passes a request without a `project_id`.
    #[must_use]
    pub fn allows(&self, project: Option<&str>) -> bool {
        match (self, project) {
            (Self::All, _) | (_, None) => true,
            (Self::Only(projects), Some(project)) => projects.contains(project),
        }
    }
}

/// One tenant's runner policy, as the feed publishes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TenantPolicy {
    pub soma: bool,
    pub suspended: bool,
    /// This runner's share of the tenant's concurrent sandboxes; `None` is unlimited.
    pub max_concurrent_share: Option<u32>,
    /// The sandbox lifetime when a create names none.
    pub default_timeout_seconds: Option<u64>,
}

/// A tenant's policy joined with its live sandbox count on this runner.
///
/// The count is shared by every policy version of the tenant and survives snapshots, so a
/// policy change never forgets the sandboxes already running.
#[derive(Debug)]
pub struct Tenant {
    pub policy: TenantPolicy,
    live: Arc<AtomicI64>,
}

impl Tenant {
    #[must_use]
    pub const fn new(policy: TenantPolicy, live: Arc<AtomicI64>) -> Self {
        Self { policy, live }
    }

    /// Counts one more live sandbox if the tenant's share allows it.
    ///
    /// Lock-free: increment, and give the slot back when the share is exceeded. Two runners
    /// cannot see each other's counts, so the tenant-wide overshoot this allows is bounded by
    /// the control plane's split of the limit (contract C3).
    #[must_use]
    pub fn admit(&self) -> bool {
        let live = self.live.fetch_add(1, Ordering::AcqRel) + 1;
        let within = self
            .policy
            .max_concurrent_share
            .is_none_or(|share| live <= i64::from(share));
        if !within {
            self.live.fetch_sub(1, Ordering::AcqRel);
        }
        within
    }

    /// The counter a sandbox gives back when it ends.
    #[must_use]
    pub fn counter(&self) -> Arc<AtomicI64> {
        Arc::clone(&self.live)
    }

    #[must_use]
    pub fn live(&self) -> i64 {
        self.live.load(Ordering::Acquire)
    }
}

/// Why a caller was not admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No `Bearer msk_` token, or a token the table does not hold (401).
    Unauthorized,
    /// The key is known but its tenant may not use the runner (403).
    Forbidden,
}

/// The identity a request acts under once its key is accepted.
#[derive(Clone, Debug)]
pub struct Principal {
    pub key: Arc<KeyRecord>,
    pub tenant: Arc<Tenant>,
}

/// One key record joined with its tenant, so admission is a single hash lookup.
#[derive(Debug)]
pub struct Joined {
    pub key: Arc<KeyRecord>,
    /// `None` until the feed publishes the tenant's policy; such a key is refused.
    pub tenant: Option<Arc<Tenant>>,
}
