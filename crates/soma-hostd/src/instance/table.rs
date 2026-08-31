//! The Instance ownership table and the bounded page a listing reports.
//!
//! Entries are keyed by [`InstanceId`] and hold no guest state, only the identities the owner
//! needs to address the Machine again.
//! The table has no capacity of its own: admission already bounds how many Instances one Host
//! may hold, and a second bound here could only refuse work capacity had already accepted.

use std::collections::BTreeMap;

use soma_guest::LaunchNetwork;

use crate::{InstanceId, LeaseGeneration, OperationId, RequestFingerprint, WorkerId};

/// What the Runtime reports about one live Instance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InstanceView {
    /// The Instance.
    pub instance: InstanceId,
    /// The worker serving it.
    pub worker: WorkerId,
    /// The lease generation the Launch won.
    pub lease_generation: LeaseGeneration,
    /// The operation that created it.
    pub operation: OperationId,
    /// The canonical fingerprint of the request that created it, which is what a replay of
    /// that operation must present again.
    pub fingerprint: RequestFingerprint,
    /// The exact launch-page network identity delivered to the worker.
    pub launch: LaunchNetwork,
}

/// One bounded page of a listing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Page {
    /// The Instances of this page, in identity order.
    pub instances: Vec<InstanceId>,
    /// Whether more live Instances follow the last one listed.
    pub more: bool,
}

/// Every Instance this Host owns.
#[derive(Default)]
pub struct LiveTable {
    entries: BTreeMap<InstanceId, InstanceView>,
}

impl LiveTable {
    /// Records one Instance-to-worker binding, replacing any binding of the same identity.
    pub(super) fn bind(&mut self, view: InstanceView) {
        self.entries.insert(view.instance, view);
    }

    /// Returns one Instance.
    pub(super) fn get(&self, instance: InstanceId) -> Option<InstanceView> {
        self.entries.get(&instance).copied()
    }

    /// Forgets one Instance and returns what it was bound to.
    pub(super) fn remove(&mut self, instance: InstanceId) -> Option<InstanceView> {
        self.entries.remove(&instance)
    }

    /// Returns how many Instances are owned.
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Keeps only the Instances whose worker `alive` proves is still serving them.
    pub(super) fn retain(&mut self, alive: impl Fn(WorkerId) -> bool) {
        self.entries.retain(|_, view| alive(view.worker));
    }

    /// Returns at most `limit` Instances ordered after `after`.
    ///
    /// Identity order is total and stable, so a client that pages with the last identity it
    /// received enumerates every Instance without holding a cursor the Host must remember.
    pub(super) fn page(&self, after: Option<InstanceId>, limit: usize) -> Page {
        let mut rest = self
            .entries
            .keys()
            .copied()
            .filter(|instance| after.is_none_or(|start| *instance > start));
        let instances: Vec<InstanceId> = rest.by_ref().take(limit).collect();
        Page {
            more: rest.next().is_some(),
            instances,
        }
    }
}
