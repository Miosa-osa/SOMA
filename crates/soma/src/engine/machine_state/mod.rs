mod active;
mod durable;
mod model;
mod tombstone;

pub(super) use model::{
    ActiveMachine, DurableMachine, DurablePhase, ExecutionTombstone, LaunchIntent, TerminalBasis,
    VersionedMachine,
};

pub(super) const MAX_EXECUTION_TOMBSTONES: usize = 1_024;

const fn corrupt() -> crate::StateStoreFailure {
    crate::StateStoreFailure::new(crate::StateStoreFailureKind::Corrupt)
}
