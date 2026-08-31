//! Semantic owner for one authenticated guest-control transport.
//!
//! Consumed host states cannot be reused for a second in-flight operation.
//! Host operations derive one absolute deadline for each bounded stage or exchange.
//! Guest operations accept caller-supplied absolute deadlines so the guest agent and VMM retain
//! ownership of sandbox lifetime policy.
//! A `ControlIo` adapter must cancel its underlying operation and return by every supplied
//! deadline.
//! Poisoning must trigger locally bounded transport cancellation and must not wait for a peer.
//!
//! ```compile_fail
//! use soma_guest::{HostControl, HostControlIo};
//!
//! fn start_twice<I: HostControlIo>(host: HostControl<I>) {
//!     let _first = host.prepare_and_probe();
//!     let _second = host.prepare_and_probe();
//! }
//! ```
//!
//! ```compile_fail
//! use soma_guest::{GuestCommand, HostControlIo, OperationId, RepairedHostControl};
//!
//! fn execute_then_reuse<I: HostControlIo>(
//!     host: RepairedHostControl<I>,
//!     operation: OperationId,
//!     command: GuestCommand,
//! ) {
//!     let _first = host.execute(operation, command);
//!     let _second = host.shutdown(operation);
//! }
//! ```

mod channel;
mod deadline;
mod error;
mod exchange;
mod guest;
mod guest_connect;
mod guest_state;
mod host;
mod host_connect;
mod io;
mod operation_ledger;
mod outcome;
mod request;

pub use error::{ControlError, ControlFailureClass, ControlStage};

/// Fixed vsock port of the SOMA control endpoint on the host context (CID 2).
///
/// The trusted guest agent connects from its assigned CID to this port on `VMADDR_CID_HOST`.
/// The VMM vsock device accepts only this destination port, and the value is part of the
/// machine contract so both peers change it together.
pub const CONTROL_VSOCK_PORT: u32 = 0x534f_4d41;
pub use guest::GuestControl;
pub use host::{HostControl, RepairedHostControl, WholeFileRead, WholeFileWrite};
pub use io::{ControlIo, HostControlIo};
pub use outcome::ExecuteOutcome;
pub use request::GuestRequest;

#[cfg(test)]
mod tests;
