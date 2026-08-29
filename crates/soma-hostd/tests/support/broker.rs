//! One resource broker shared by every pool of a test, so a simulated restart still faces
//! the leases and bundles the crashed process left on the host.

use std::{sync::Arc, time::Duration};

use soma_hostd::{
    AssignedResources, AssignmentIntent, PoolKey, ResourceBroker, ResourceFault, ResourceLiveness,
    ResourceRefs, ResourceRelease, WorkerId,
    testing::{InProcessBroker, SterileResources},
};

/// A broker every pool of one test shares.
#[derive(Clone, Debug)]
pub struct SharedBroker(Arc<InProcessBroker>);

impl SharedBroker {
    /// Wraps the host broker.
    pub fn new(broker: &Arc<InProcessBroker>) -> Self {
        Self(Arc::clone(broker))
    }
}

impl ResourceBroker for SharedBroker {
    type Sterile = SterileResources;

    fn prepare(
        &self,
        key: &PoolKey,
        worker: WorkerId,
        budget: Duration,
    ) -> Result<(Self::Sterile, ResourceRefs), ResourceFault> {
        self.0.prepare(key, worker, budget)
    }

    fn assign(
        &self,
        sterile: Self::Sterile,
        intent: &AssignmentIntent,
    ) -> Result<AssignedResources, ResourceFault> {
        self.0.assign(sterile, intent)
    }

    fn release_sterile(&self, sterile: Self::Sterile) -> ResourceRelease {
        self.0.release_sterile(sterile)
    }

    fn release(&self, refs: &ResourceRefs) -> ResourceRelease {
        self.0.release(refs)
    }

    fn verify(&self, refs: &ResourceRefs) -> ResourceLiveness {
        self.0.verify(refs)
    }
}
