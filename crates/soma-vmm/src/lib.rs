#![deny(warnings)]
#![deny(unsafe_code)]

pub mod control;

mod error;
mod ids;
mod machine;
mod operation;
mod platform;
mod receipt;
mod request;
mod spec;

pub use error::{CleanupEvidence, Failure, FailureKind, FailurePhase, Recovery};
pub use ids::{GenerationId, IdError, InstanceId, OperationId};
pub use machine::Machine;
pub use operation::{MAX_OPERATION_RECEIPT_BYTES, MAX_OPERATION_RECEIPTS};
pub use receipt::{Executed, ExitStatus, Milestone, Milestones, Ready, Stopped};
pub use request::{
    Argument, CommandError, Execute, ExecutionLimits, Launch, OutputBytes, Program, Stop,
    TimeoutMillis,
};
pub use spec::{DiskBytes, Generation, MachineSpec, MemoryBytes, SpecError, VcpuCount};

#[cfg(test)]
mod tests;
