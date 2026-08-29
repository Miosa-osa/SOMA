//! Node-local capacity admission from the visual atlas capacity model.
//!
//! [`HostProfile`] declares inventory, host reserve, measured per-VM overhead, per-dimension
//! limits, and the per-class overcommit policy, and certifies into the [`CertifiedProfile`]
//! every entry point takes; [`Admission`] reserves every dimension
//! atomically with checked arithmetic and rolls back on the first refusing gate; and
//! [`estimate`] reproduces the capacity ladder arithmetic for one uniform shape.

pub mod capacity;
pub mod demand;
pub mod numa;
pub mod overhead;
pub mod profile;
pub mod rejection;
pub mod reserve;
pub mod shape;
pub mod usage;

pub use capacity::{CapacityEstimate, estimate};
pub use demand::Demand;
pub use numa::{NodeDemand, NodeFree, NodeId, NumaPlacement, NumaRejection, SingleNode};
pub use overhead::MeasuredOverhead;
pub use profile::{
    CertifiedProfile, CpuInventory, HostProfile, MemoryInventory, NetworkInventory, OperatorLimits,
    OvercommitPolicy, ProcessInventory, ProfileError, Ratio, StorageInventory,
};
pub use rejection::{CapacityRejection, Gate};
pub use reserve::{Admission, Reservation};
pub use shape::{MachineShape, MemoryClass, ShapeError, ValidShape, WorkloadClass};
pub use usage::Usage;
