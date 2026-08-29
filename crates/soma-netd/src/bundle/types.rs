//! The sterile and assigned bundle handles.

use std::os::fd::OwnedFd;

use soma_guest::LaunchNetwork;

use crate::{
    AssignmentRecord, BundleId, BundleNames, CleanupGeneration, ConntrackZone, Error, LeasePair,
    MacPair, PortReservation, namespace::NetNamespace,
};

/// One prepared, unassigned bundle.
#[derive(Debug)]
pub struct SterileBundle {
    pub(crate) id: BundleId,
    pub(crate) generation: CleanupGeneration,
    pub(crate) names: BundleNames,
    pub(crate) namespace: NetNamespace,
    pub(crate) tap: OwnedFd,
    pub(crate) leases: LeasePair,
    pub(crate) macs: MacPair,
    pub(crate) zone: ConntrackZone,
}

impl SterileBundle {
    /// Returns the bundle identity.
    #[must_use]
    pub const fn id(&self) -> BundleId {
        self.id
    }

    /// Returns the cleanup generation.
    #[must_use]
    pub const fn generation(&self) -> CleanupGeneration {
        self.generation
    }

    /// Returns the kernel names.
    #[must_use]
    pub const fn names(&self) -> &BundleNames {
        &self.names
    }

    /// Returns the pinned namespace.
    #[must_use]
    pub const fn namespace(&self) -> &NetNamespace {
        &self.namespace
    }

    /// Returns the lease pair.
    #[must_use]
    pub const fn leases(&self) -> LeasePair {
        self.leases
    }

    /// Returns the MAC pair.
    #[must_use]
    pub const fn macs(&self) -> MacPair {
        self.macs
    }

    /// Returns the conntrack zone.
    #[must_use]
    pub const fn zone(&self) -> ConntrackZone {
        self.zone
    }

    /// Returns the TAP descriptor for transfer; the bundle keeps ownership.
    #[must_use]
    pub const fn tap(&self) -> &OwnedFd {
        &self.tap
    }
}

/// One bundle bound to an Instance.
#[derive(Debug)]
pub struct Assigned {
    pub(crate) bundle: SterileBundle,
    pub(crate) record: AssignmentRecord,
    pub(crate) launch: LaunchNetwork,
    pub(crate) reservations: Vec<PortReservation>,
    pub(crate) active: bool,
}

impl Assigned {
    /// Returns the underlying bundle.
    #[must_use]
    pub const fn bundle(&self) -> &SterileBundle {
        &self.bundle
    }

    /// Returns the durable record.
    #[must_use]
    pub const fn record(&self) -> &AssignmentRecord {
        &self.record
    }

    /// Returns the exact launch-page network values.
    #[must_use]
    pub const fn launch(&self) -> LaunchNetwork {
        self.launch
    }

    /// Returns the held port reservations.
    #[must_use]
    pub fn reservations(&self) -> &[PortReservation] {
        &self.reservations
    }

    /// Returns whether forwarding has been activated.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active
    }
}

/// An assignment failure that hands the sterile bundle back for release.
#[derive(Debug)]
pub struct AssignFailure {
    /// The bundle that must still be released.
    pub bundle: Box<SterileBundle>,
    /// The typed failure.
    pub error: Error,
}
