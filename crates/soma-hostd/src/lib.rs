//! `soma-hostd`: the node-local host allocator accepted by ADR 0006.
//!
//! The crate owns bounded pools of sterile single-use workers and resource bundles keyed by
//! the exact [`PoolKey`], the single-winner idempotent claim, the exactly-once transfer of
//! fresh per-Instance authority, bounded replenishment and backpressure, the durable
//! lifecycle ledger, restart reconciliation, and the multi-dimension capacity admission of
//! the visual atlas, which every claim reserves from before a worker is granted.
//! It never executes guest device logic and never returns an assigned worker to a pool.
//!
//! The [`Runtime`] adds the persistent Instance ownership of ADR 0031 on top of that pool:
//! it owns every live Instance of one Host, so an Instance is addressable by identity long
//! after the client that launched it has gone.
//!
//! The Linux-only `client` module is the other half of the same protocol: an adapter links this crate
//! and addresses a Machine by identity from any process, so the encoding never exists twice.
//!
//! The jail launcher, the storage clone path, and the network broker are consumed through
//! the [`WorkerLauncher`] and [`ResourceBroker`] seams; the in-process implementations in
//! [`testing`] make every policy testable without a kernel.

#![deny(unsafe_code)]

pub mod admission;
pub mod ids;
pub mod instance;
pub mod pool;
pub mod protocol;

#[cfg(unix)]
pub mod testing;

#[cfg(target_os = "linux")]
pub mod client;

#[cfg(target_os = "linux")]
pub mod daemon;

pub use instance::{
    InstanceError, InstanceView, Launched, MAX_LISTED, Page, Runtime, TerminalReceipt,
};
pub use protocol::{
    FailureCode, LaunchFrame, MAX_FRAME, ProtocolError, Reply, Request, claim_failure_code,
    failure_code, instance_failure_code, lifecycle_failure_code, transfer_failure_code,
};

pub use admission::{
    Admission, CapacityEstimate, CapacityRejection, CertifiedProfile, CpuInventory, Demand, Gate,
    HostProfile, MachineShape, MeasuredOverhead, MemoryClass, MemoryInventory, NetworkInventory,
    NodeDemand, NodeFree, NodeId, NumaPlacement, NumaRejection, OperatorLimits, OvercommitPolicy,
    ProcessInventory, ProfileError, Ratio, Reservation, ShapeError, SingleNode, StorageInventory,
    Usage, ValidShape, WorkloadClass, estimate, reserve,
};
pub use ids::{
    GenerationId, HostProfileDigest, IdError, InstanceId, LaunchMaterialHandle, LeaseGeneration,
    OperationId, RequestFingerprint, WorkerId,
};
pub use pool::{
    Pool, PoolError, WorkerView,
    backpressure::{
        Exhausted, ExhaustedBehavior, Limits, LimitsError, Occupancy, OverloadGate, Overloaded,
    },
    capacity::PoolAdmission,
    claim::{Claim, ClaimClass, ClaimError, ClaimOutcome, Claimed},
    key::{CpuClass, MemoryShape, OverlayIdentity, PoolKey, PoolKeyDigest},
    launcher::{
        ConstructFault, DestroyOutcome, Liveness, Removal, StartFault, WorkerHandle,
        WorkerIdentity, WorkerLauncher,
    },
    ledger::{ClaimRecord, Ledger, LedgerError, RECORD_LEN, Record, RecordKind},
    reconcile::{ReconcileDisposition, ReconcileFinding, ReconcileReport},
    release::{DestroyReason, LifecycleError, ReleaseEvidence},
    replenish::{ConstructionFailure, ConstructionFault, ReplenishLimit, ReplenishReport},
    resources::{
        AssignedResources, AssignmentIntent, ControlGrant, Descriptor, DiskGrant, NetworkGrant,
        Resource, ResourceBroker, ResourceFault, ResourceFaultKind, ResourceLiveness, ResourceRefs,
        ResourceRelease,
    },
    state::{
        Assigned, Claiming, Constructing, Dead, Destroying, Packed, Phase, Phased, Running, Slot,
        StateRace, StateWord, Sterile, Worker, WorkerLedgerEntry,
    },
    transfer::{
        Disposition, StepAck, TransferEvidence, TransferFailure, TransferFault, TransferFrame,
        TransferStep,
    },
};
