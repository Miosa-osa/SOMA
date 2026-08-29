use crate::MemoryStateStore;

mod machine_state;
mod managed;
mod managed_execute;
mod managed_inspection;
mod managed_launch;
mod managed_receipt;
mod managed_state;
mod managed_termination;
mod managed_types;
mod run;
mod run_cleanup;
mod run_evidence;
mod run_types;

pub use managed::{ManagedFailure, ManagedStateError, ReplayEvidence};
pub use managed_types::{
    DestroyMachineRequest, ExecuteMachineRequest, InspectMachineRequest, LaunchMachineRequest,
    MachineDestroy, MachineExecution, MachineInspection, MachineLaunch, MachineStop,
    StopMachineRequest,
};
pub use run_types::{FailurePhase, RunFailure, RunFailureKind, RunOutcome};

pub struct Engine<B, S = MemoryStateStore> {
    backend: B,
    state: S,
}

impl<B> Engine<B, MemoryStateStore> {
    #[must_use]
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            state: MemoryStateStore::new(),
        }
    }
}

impl<B, S> Engine<B, S> {
    #[must_use]
    pub const fn with_state_store(backend: B, state: S) -> Self {
        Self { backend, state }
    }
}
