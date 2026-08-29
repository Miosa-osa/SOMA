//! The capacity gate every claim of one pool passes before a worker is granted.
//!
//! A pool is opened against the host [`Admission`] and the exact Machine shape its sterile
//! workers were prepared for, so [`Pool::claim`] reserves every visual-atlas dimension
//! atomically before it wins a slot and answers a refusal with the typed
//! [`CapacityRejection`] naming the gate.
//! The reservation travels with the worker: the transfer marks the Launch finished and every
//! teardown, including the one a restart performs, returns it.

use std::sync::Arc;

use crate::{
    Admission, CapacityRejection, Pool, Reservation, ResourceBroker, ValidShape, WorkerLauncher,
};

/// The host admission one pool reserves from, and the shape it reserves.
#[derive(Clone)]
pub struct PoolAdmission {
    admission: Arc<Admission>,
    shape: ValidShape,
}

impl PoolAdmission {
    /// Binds one pool's Machine shape to the host admission every pool shares.
    #[must_use]
    pub const fn new(admission: Arc<Admission>, shape: ValidShape) -> Self {
        Self { admission, shape }
    }

    /// The host admission.
    #[must_use]
    pub const fn admission(&self) -> &Arc<Admission> {
        &self.admission
    }

    /// The shape every claim of this pool reserves.
    #[must_use]
    pub const fn shape(&self) -> &ValidShape {
        &self.shape
    }
}

impl<L: WorkerLauncher, R: ResourceBroker> Pool<L, R> {
    /// The host admission this pool reserves from.
    #[must_use]
    pub const fn capacity(&self) -> &PoolAdmission {
        &self.capacity
    }

    /// Reserves every capacity dimension for one Instance of this pool's shape.
    pub(crate) fn reserve_capacity(&self) -> Result<Reservation, CapacityRejection> {
        self.capacity.admission.reserve(&self.capacity.shape)
    }

    /// Returns a reservation to the host admission.
    pub(crate) fn release_capacity(&self, reservation: Option<Reservation>) {
        if let Some(reservation) = reservation {
            self.capacity.admission.release(reservation, None);
        }
    }

    /// Marks the Launch of `reservation` finished so its launch slot is free.
    pub(crate) fn capacity_launched(&self, reservation: &mut Option<Reservation>) {
        if let Some(reservation) = reservation.as_mut() {
            self.capacity.admission.launched(reservation);
        }
    }
}
