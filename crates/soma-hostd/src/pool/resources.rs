//! The seam between the pool and the brokers that prepare sterile disk heads and network
//! bundles and turn them into fresh per-Instance authority.
//!
//! Disk heads come from `soma-storage` leases and network bundles from `soma-netd`
//! assignments; the pool never learns a path, a TAP name, or a ruleset.

use std::{fmt, time::Duration};

use soma_guest::LaunchNetwork;
use soma_netd::{BundleId, CleanupGeneration, NetworkIntent};
use soma_storage::{HeadToken, LeaseReceipt};

use crate::{InstanceId, LaunchMaterialHandle, OperationId, PoolKey, RequestFingerprint, WorkerId};

/// An open descriptor that is transferred into the worker.
#[cfg(unix)]
pub type Descriptor = std::os::fd::OwnedFd;

/// No descriptor can exist on a non-Unix host, so no authority can be transferred there.
#[cfg(not(unix))]
#[derive(Debug)]
pub enum Descriptor {}

/// Everything a caller binds to one claim; its fingerprint is the replay identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssignmentIntent {
    /// The fresh Instance.
    pub instance: InstanceId,
    /// The Launch operation.
    pub operation: OperationId,
    /// The vsock CID the host allocated.
    pub vsock_cid: u32,
    /// The admitted network intent.
    pub network: NetworkIntent,
    /// How long the Instance may live.
    pub deadline: Duration,
    /// The sealed launch material the worker will receive.
    pub launch_material: LaunchMaterialHandle,
}

impl AssignmentIntent {
    /// Encodes the intent canonically.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let network = self.network.encode();
        let mut out = Vec::with_capacity(76 + network.len());
        out.extend_from_slice(self.instance.as_bytes());
        out.extend_from_slice(self.operation.as_bytes());
        out.extend_from_slice(&self.vsock_cid.to_be_bytes());
        out.extend_from_slice(&self.deadline_nanos().to_be_bytes());
        out.extend_from_slice(self.launch_material.as_bytes());
        out.extend_from_slice(&network);
        out
    }

    /// The replay identity of the intent.
    #[must_use]
    pub fn fingerprint(&self) -> RequestFingerprint {
        RequestFingerprint::of(&self.encode())
    }

    /// The deadline in nanoseconds, saturated.
    #[must_use]
    pub fn deadline_nanos(&self) -> u64 {
        u64::try_from(self.deadline.as_nanos()).unwrap_or(u64::MAX)
    }
}

/// One leased private disk head.
#[derive(Debug)]
pub struct DiskGrant {
    /// The single-use lease from `soma-storage`.
    pub receipt: LeaseReceipt,
    /// The open head.
    pub head: Descriptor,
}

/// One assigned network bundle.
#[derive(Debug)]
pub struct NetworkGrant {
    /// The bundle.
    pub bundle: BundleId,
    /// Its cleanup generation.
    pub generation: CleanupGeneration,
    /// The exact launch-page values.
    pub launch: LaunchNetwork,
    /// The open TAP.
    pub tap: Descriptor,
}

/// The control authority of one Instance.
#[derive(Debug)]
pub struct ControlGrant {
    /// The vsock CID.
    pub vsock_cid: u32,
    /// The worker end of the control channel.
    pub channel: Descriptor,
}

/// The fresh authority bundle that is transferred exactly once.
#[derive(Debug)]
pub struct AssignedResources {
    /// Disk.
    pub disk: DiskGrant,
    /// Network.
    pub network: NetworkGrant,
    /// Control.
    pub control: ControlGrant,
}

impl AssignedResources {
    /// The durable references release and reconciliation use.
    #[must_use]
    pub fn refs(&self) -> ResourceRefs {
        ResourceRefs {
            head: Some(self.disk.receipt.token()),
            bundle: Some((self.network.bundle, self.network.generation)),
        }
    }
}

/// Durable references to the resources one worker holds.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResourceRefs {
    /// The head lease token.
    pub head: Option<HeadToken>,
    /// The network bundle and generation.
    pub bundle: Option<(BundleId, CleanupGeneration)>,
}

impl ResourceRefs {
    /// Encoded length.
    pub const LEN: usize = 36;

    /// Encodes the references; absent parts are zero.
    #[must_use]
    pub fn encode(&self) -> [u8; Self::LEN] {
        let mut out = [0; Self::LEN];
        if let Some(head) = self.head {
            out[..16].copy_from_slice(head.as_bytes());
        }
        if let Some((bundle, generation)) = self.bundle {
            out[16..32].copy_from_slice(bundle.as_bytes());
            out[32..].copy_from_slice(&generation.get().to_be_bytes());
        }
        out
    }

    /// Decodes references; zero parts are absent.
    #[must_use]
    pub fn decode(bytes: &[u8; Self::LEN]) -> Self {
        let mut head = [0; 16];
        head.copy_from_slice(&bytes[..16]);
        let mut bundle = [0; 16];
        bundle.copy_from_slice(&bytes[16..32]);
        let generation = u32::from_be_bytes([bytes[32], bytes[33], bytes[34], bytes[35]]);
        Self {
            head: HeadToken::new(head).ok(),
            bundle: BundleId::new(bundle)
                .ok()
                .zip(CleanupGeneration::new(generation).ok()),
        }
    }
}

/// Which resource a fault concerns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Resource {
    /// The private disk head.
    Disk,
    /// The network bundle.
    Network,
    /// The control channel.
    Control,
}

/// What went wrong with a resource.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceFaultKind {
    /// The broker's bounded inventory is empty.
    Exhausted,
    /// The broker refused the intent.
    Denied,
    /// The broker did not answer in time.
    Timeout,
    /// The single-use lease was refused.
    Lease,
    /// A mechanism failed.
    Failed,
}

/// One typed resource fault.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceFault {
    /// The resource.
    pub resource: Resource,
    /// The fault.
    pub kind: ResourceFaultKind,
}

impl fmt::Display for ResourceFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?} resource {:?}", self.resource, self.kind)
    }
}

impl std::error::Error for ResourceFault {}

/// What a verification of recorded references found.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceLiveness {
    /// Every referenced resource exists.
    Present,
    /// No referenced resource exists.
    Absent,
    /// Some referenced resources exist.
    Partial,
}

/// What releasing resources did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceRelease {
    /// The head.
    pub disk: crate::Removal,
    /// The bundle.
    pub network: crate::Removal,
    /// Whether nothing referenced remains.
    pub complete: bool,
}

/// Prepares sterile resources and assigns them to exactly one Instance.
pub trait ResourceBroker: Send + Sync + 'static {
    /// One prepared, unassigned resource set.
    type Sterile: Send + 'static;

    /// Prepares sterile resources for `worker` within `budget`.
    ///
    /// # Errors
    ///
    /// Returns the typed fault with nothing left behind.
    fn prepare(
        &self,
        key: &PoolKey,
        worker: WorkerId,
        budget: Duration,
    ) -> Result<(Self::Sterile, ResourceRefs), ResourceFault>;

    /// Leases the head and assigns the bundle to the Instance in `intent`.
    ///
    /// # Errors
    ///
    /// Returns the typed fault; the broker releases whatever it consumed first.
    fn assign(
        &self,
        sterile: Self::Sterile,
        intent: &AssignmentIntent,
    ) -> Result<AssignedResources, ResourceFault>;

    /// Releases sterile resources that were never assigned.
    fn release_sterile(&self, sterile: Self::Sterile) -> ResourceRelease;

    /// Releases resources by durable reference after transfer or after a restart.
    fn release(&self, refs: &ResourceRefs) -> ResourceRelease;

    /// Verifies recorded references against the brokers' ledgers.
    fn verify(&self, refs: &ResourceRefs) -> ResourceLiveness;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs_round_trip_with_absent_parts() {
        let refs = ResourceRefs {
            head: Some(HeadToken::new([1; 16]).expect("token")),
            bundle: Some((
                BundleId::new([2; 16]).expect("bundle"),
                CleanupGeneration::new(3).expect("generation"),
            )),
        };
        assert_eq!(ResourceRefs::decode(&refs.encode()), refs);
        assert_eq!(
            ResourceRefs::decode(&[0; ResourceRefs::LEN]),
            ResourceRefs::default()
        );
    }
}
