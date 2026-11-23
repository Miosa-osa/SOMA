//! The packet protocol one jailed VMM worker serves over its control descriptor.
//!
//! A worker inside the jail has no filesystem, no environment, and no argument beyond its
//! descriptor manifest, so its supervisor drives the lifecycle by writing one request packet
//! to the control socket and reading one reply packet back.
//!
//! The packets carry this crate's contract types unchanged: a request decodes into a
//! [`Launch`](crate::Launch), an [`Execute`](crate::Execute), or a [`Stop`](crate::Stop), and
//! a reply is the encoding of the [`Ready`](crate::Ready), [`Executed`](crate::Executed),
//! [`Stopped`](crate::Stopped), or [`Failure`](crate::Failure) the Machine produced.
//! The encoding is one line of ASCII so that a supervisor, a live test, and a log all read the
//! same bytes.

mod error;
mod field;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod files;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use files::{
    FILE_CONTROL_CEILING, read_file_answer, read_file_request, write_file_answer,
    write_file_request,
};
mod outcome;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod pty;
mod reply;
mod request;
mod window;

pub use error::ControlError;
pub use outcome::Outcome;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use pty::PtyRequest;
pub use reply::Reply;
pub use request::{MAX_REQUEST_BYTES, Request};
pub use window::{MAX_OUTPUT_WINDOW_BYTES, MAX_REPLY_BYTES, OutputStream, OutputWindow};

/// The version of this packet protocol.
///
/// No packet carries it. The supervisor and the worker it executes always come from one
/// installed release, because the worker is started from that release's binary for each launch,
/// so the two ends never differ. The number names the packet layout in `--version` and in the
/// compatibility table of `docs/product/reference/versioning.md`, and it changes whenever a packet's layout does.
pub const CONTROL_PROTOCOL_VERSION: u16 = 3;
