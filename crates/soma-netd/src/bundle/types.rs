//! The sterile and assigned bundle handles.

use std::os::fd::OwnedFd;

use soma_guest::{ActivationChallenge, ActivationReceipt, LaunchNetwork};

use crate::{
    AssignmentRecord, BundleId, BundleNames, CleanupGeneration, ConntrackZone, Error, LeasePair,
    MacPair, PortReservation, PublishedPort, namespace::NetNamespace,
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
    pub(crate) published: Vec<PublishedPort>,
    pub(crate) activation: Option<ActivationChallenge>,
    pub(crate) activated: Option<ActivationReceipt>,
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

    /// Returns the destination mappings this assignment will publish at activation.
    #[must_use]
    pub fn published(&self) -> &[PublishedPort] {
        &self.published
    }

    /// Returns whether forwarding has been activated.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active
    }

    /// Returns whether this exact receipt is the one that already activated this assignment.
    ///
    /// A peer whose `Activated` reply was lost replays the same request; answering it from the
    /// recorded receipt keeps activation idempotent under the operation identity instead of
    /// destroying a Machine that is already running.
    #[must_use]
    pub fn activated_by(&self, receipt: &ActivationReceipt) -> bool {
        self.activated.is_some_and(|prior| prior == *receipt)
    }

    /// Borrows the single-use activation challenge until one activation attempt consumes it.
    ///
    /// The broker delivers these bytes only to the peer that claimed this assignment; the
    /// repaired guest session converts them into the receipt [`crate::activate`] requires.
    #[must_use]
    pub const fn activation_challenge(&self) -> Option<&ActivationChallenge> {
        self.activation.as_ref()
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
