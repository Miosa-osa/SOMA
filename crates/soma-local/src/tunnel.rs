//! A byte stream from the runner to one port of a hosted sandbox's guest.
//!
//! The guest's network lives in a namespace only the privileged broker can enter, and the process
//! that holds the sandbox is the one that holds its network lease. So the stream is opened by
//! asking that process: it has the broker dial the guest from inside the namespace, then answers
//! and pumps bytes between the caller's connection and the guest's.

/// Why a tunnel could not be opened.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TunnelFailure {
    /// No hosted sandbox with this identity is reachable on this host.
    NotFound,
    /// The sandbox has no network, so there is no guest address to dial.
    NoNetwork,
    /// The port is not one the broker may dial.
    PortNotAllowed,
    /// The guest did not accept the connection in time.
    Unreachable,
    /// The exchange failed for any other reason.
    Unavailable,
}
