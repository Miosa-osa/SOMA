//! The in-process resource broker: real `soma-storage` head leases over socket pairs and
//! derived launch identities in place of TAP devices.

mod launch;

use std::{
    collections::BTreeSet,
    os::fd::OwnedFd,
    sync::{
        Mutex,
        atomic::{AtomicU32, AtomicUsize, Ordering},
    },
    time::Duration,
};

use soma_netd::{BundleId, CleanupGeneration};
use soma_storage::{HeadLedger, HeadName, HeadToken};

use launch::{launch_identity, pair, sterile_token};

use crate::{
    AssignedResources, AssignmentIntent, ControlGrant, DiskGrant, NetworkGrant, PoolKey, Removal,
    Resource, ResourceBroker, ResourceFault, ResourceFaultKind, ResourceLiveness, ResourceRefs,
    ResourceRelease, WorkerId,
};

/// One prepared, unassigned resource set.
#[derive(Debug)]
pub struct SterileResources {
    head: OwnedFd,
    tap: OwnedFd,
    control: OwnedFd,
    bundle: BundleId,
    name: HeadName,
}

/// Counters the tests assert on.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BrokerCounters {
    /// Sterile sets prepared.
    pub prepared: usize,
    /// Sets assigned.
    pub assigned: usize,
    /// Sterile sets released.
    pub released_sterile: usize,
    /// Assigned sets released by reference.
    pub released: usize,
}

/// The in-process broker.
#[derive(Debug)]
pub struct InProcessBroker {
    heads: Mutex<HeadLedger>,
    bundles: Mutex<BTreeSet<BundleId>>,
    generation: CleanupGeneration,
    lease_index: AtomicU32,
    prepared: AtomicUsize,
    assigned: AtomicUsize,
    released_sterile: AtomicUsize,
    released: AtomicUsize,
    fault: Mutex<Option<ResourceFault>>,
}

impl Default for InProcessBroker {
    fn default() -> Self {
        Self::new()
    }
}

impl InProcessBroker {
    /// An empty broker.
    #[must_use]
    pub fn new() -> Self {
        Self {
            heads: Mutex::new(HeadLedger::new()),
            bundles: Mutex::new(BTreeSet::new()),
            generation: CleanupGeneration::new(1).unwrap_or_else(|_| unreachable!("nonzero")),
            lease_index: AtomicU32::new(0),
            prepared: AtomicUsize::new(0),
            assigned: AtomicUsize::new(0),
            released_sterile: AtomicUsize::new(0),
            released: AtomicUsize::new(0),
            fault: Mutex::new(None),
        }
    }

    /// Makes every later `assign` fail with `fault`.
    pub fn fail_assign(&self, fault: Option<ResourceFault>) {
        *self
            .fault
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = fault;
    }

    /// Returns the counters.
    #[must_use]
    pub fn counters(&self) -> BrokerCounters {
        BrokerCounters {
            prepared: self.prepared.load(Ordering::Acquire),
            assigned: self.assigned.load(Ordering::Acquire),
            released_sterile: self.released_sterile.load(Ordering::Acquire),
            released: self.released.load(Ordering::Acquire),
        }
    }

    /// Live bundles, sterile or assigned.
    #[must_use]
    pub fn live_bundles(&self) -> usize {
        self.bundles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Heads currently leased to an Instance.
    #[must_use]
    pub fn leased_heads(&self) -> usize {
        self.heads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .assigned_count()
    }
}

impl ResourceBroker for InProcessBroker {
    type Sterile = SterileResources;

    fn prepare(
        &self,
        _key: &PoolKey,
        worker: WorkerId,
        _budget: Duration,
    ) -> Result<(Self::Sterile, ResourceRefs), ResourceFault> {
        let bundle = BundleId::new(*worker.as_bytes()).map_err(|_| ResourceFault {
            resource: Resource::Network,
            kind: ResourceFaultKind::Failed,
        })?;
        let sterile = SterileResources {
            head: pair(Resource::Disk)?,
            tap: pair(Resource::Network)?,
            control: pair(Resource::Control)?,
            bundle,
            name: sterile_token(worker).head_name(),
        };
        self.bundles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(bundle);
        self.prepared.fetch_add(1, Ordering::AcqRel);
        Ok((
            sterile,
            ResourceRefs {
                head: Some(sterile_token(worker)),
                bundle: Some((bundle, self.generation)),
            },
        ))
    }

    fn assign(
        &self,
        sterile: Self::Sterile,
        intent: &AssignmentIntent,
    ) -> Result<AssignedResources, ResourceFault> {
        if let Some(fault) = *self
            .fault
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            let _ = self.release_sterile(sterile);
            return Err(fault);
        }
        let token = HeadToken::new(*intent.instance.as_bytes()).map_err(|_| ResourceFault {
            resource: Resource::Disk,
            kind: ResourceFaultKind::Lease,
        })?;
        let receipt = self
            .heads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .lease(token, sterile.name.clone())
            .map_err(|_| ResourceFault {
                resource: Resource::Disk,
                kind: ResourceFaultKind::Lease,
            });
        let receipt = match receipt {
            Ok(receipt) => receipt,
            Err(fault) => {
                let _ = self.release_sterile(sterile);
                return Err(fault);
            }
        };
        let index = self.lease_index.fetch_add(1, Ordering::AcqRel);
        let launch = launch_identity(sterile.bundle, self.generation, intent.vsock_cid, index)?;
        self.assigned.fetch_add(1, Ordering::AcqRel);
        Ok(AssignedResources {
            disk: DiskGrant {
                receipt,
                head: sterile.head,
            },
            network: NetworkGrant {
                bundle: sterile.bundle,
                generation: self.generation,
                launch,
                tap: sterile.tap,
            },
            control: ControlGrant {
                vsock_cid: intent.vsock_cid,
                channel: sterile.control,
            },
        })
    }

    fn release_sterile(&self, sterile: Self::Sterile) -> ResourceRelease {
        let removed = self
            .bundles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&sterile.bundle);
        drop(sterile);
        self.released_sterile.fetch_add(1, Ordering::AcqRel);
        ResourceRelease {
            disk: Removal::Removed,
            network: if removed {
                Removal::Removed
            } else {
                Removal::AlreadyAbsent
            },
            complete: true,
        }
    }

    fn release(&self, refs: &ResourceRefs) -> ResourceRelease {
        let disk = match refs.head {
            Some(token) => match self
                .heads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .release(token)
            {
                Ok(_) => Removal::Removed,
                Err(_) => Removal::AlreadyAbsent,
            },
            None => Removal::AlreadyAbsent,
        };
        let network = match refs.bundle {
            Some((bundle, _))
                if self
                    .bundles
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&bundle) =>
            {
                Removal::Removed
            }
            _ => Removal::AlreadyAbsent,
        };
        self.released.fetch_add(1, Ordering::AcqRel);
        ResourceRelease {
            disk,
            network,
            complete: true,
        }
    }

    fn verify(&self, refs: &ResourceRefs) -> ResourceLiveness {
        let head = refs.head.is_some_and(|token| {
            self.heads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .assigned_name(token)
                .is_some()
        });
        let bundle = refs.bundle.is_some_and(|(bundle, _)| {
            self.bundles
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains(&bundle)
        });
        match (head, bundle) {
            (true, true) => ResourceLiveness::Present,
            (false, false) => ResourceLiveness::Absent,
            _ => ResourceLiveness::Partial,
        }
    }
}
