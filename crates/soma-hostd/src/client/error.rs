//! Why a client call did not produce an answer this client can act on.
//!
//! Every variant separates a fact about the connection from a fact about the Host, because a
//! caller that cannot tell "nothing is listening here" from "the Host refused this Instance"
//! cannot decide whether it may own the lifecycle itself or must report a failure.

use std::fmt;

use crate::FailureCode;

/// The typed refusal of one client call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientError {
    /// The path cannot be expressed as a Unix socket address.
    SocketPath,
    /// The socket could not be created, with the errno.
    Socket(i32),
    /// Nothing accepted a connection on that path, with the errno.
    ///
    /// This is the one variant that means no Host Runtime is serving that path, so it is the
    /// only one a caller may read as permission to own the lifecycle itself.
    Connect(i32),
    /// The request frame could not be sent, with the errno.
    Send(i32),
    /// No reply frame arrived, with the errno, or zero when the daemon closed the connection.
    Receive(i32),
    /// The daemon answered a frame this client cannot decode.
    Protocol,
    /// The daemon answered a well formed reply that does not answer the request that was
    /// sent, which is a Host defect rather than a refusal of the work.
    Unexpected,
    /// The Host refused the operation with a code this client knows.
    Refused(FailureCode),
    /// The Host refused the operation with a code this client does not know, which is what a
    /// newer daemon serving an older client looks like.
    UnknownFailure(u16),
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SocketPath => formatter.write_str("socket path is unusable"),
            Self::Socket(errno) => write!(formatter, "socket failed with errno {errno}"),
            Self::Connect(errno) => write!(formatter, "connect failed with errno {errno}"),
            Self::Send(errno) => write!(formatter, "send failed with errno {errno}"),
            Self::Receive(errno) => write!(formatter, "receive failed with errno {errno}"),
            Self::Protocol => formatter.write_str("the reply frame is malformed"),
            Self::Unexpected => formatter.write_str("the reply does not answer the request"),
            Self::Refused(code) => write!(formatter, "the Host refused with {code:?}"),
            Self::UnknownFailure(code) => {
                write!(formatter, "the Host refused with unknown code {code}")
            }
        }
    }
}

impl std::error::Error for ClientError {}

/// Turns one failure reply into the typed refusal it names.
pub(super) const fn refused(code: u16) -> ClientError {
    match FailureCode::from_wire(code) {
        Some(code) => ClientError::Refused(code),
        None => ClientError::UnknownFailure(code),
    }
}
