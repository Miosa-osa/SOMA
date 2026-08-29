#![doc = "Bounded stdio Model Context Protocol access to SOMA sandboxes."]
#![forbid(unsafe_code)]

mod control;
mod identity;
mod input;
mod local;
mod outcome;
mod request;
mod result;
mod runtime;
mod server;
mod stdio_transport;

pub use control::{DestroyRequest, InspectRequest, StopRequest};
pub use identity::{InstanceId, OperationId};
pub use local::LocalToolRuntime;
pub use outcome::{
    CommandResult, CommandStatus, DoctorReport, DoctorStatus, ExecutionReceipt, InspectResult,
    MachineResult, MachineState, ReceiptValidationError,
};
pub use request::{
    BackendTarget, DirectCommand, DisplayName, ExecRequest, ExecutionLimits, LaunchRequest,
    MachineShape, OciImage, RunRequest,
};
pub use runtime::{
    RuntimeFailure, RuntimeFailureKind, RuntimeRequest, RuntimeResponse, ToolRuntime,
    UnavailableRuntime,
};
pub use server::SomaMcpServer;
pub use stdio_transport::{BoundedStdioTransport, MAX_INBOUND_MESSAGE_BYTES, bounded_stdio};
