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
mod reply;
mod request;
mod window;

pub use error::ControlError;
pub use reply::Reply;
pub use request::{MAX_REQUEST_BYTES, Request};
pub use window::{MAX_OUTPUT_WINDOW_BYTES, OutputStream, OutputWindow};
