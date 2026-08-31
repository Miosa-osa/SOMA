use std::{error::Error, fmt};

/// Why a control packet was refused.
///
/// A refusal never changes Machine state, so the supervisor may correct the packet and send it
/// again on the same connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlError {
    /// The packet is longer than [`MAX_REQUEST_BYTES`](super::MAX_REQUEST_BYTES).
    TooLong,
    UnknownRequest,
    MissingField(&'static str),
    InvalidValue(&'static str),
    /// The request named more fields than its form takes.
    TrailingField,
}

impl fmt::Display for ControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong => write!(formatter, "request exceeds the packet limit"),
            Self::UnknownRequest => write!(formatter, "unknown request"),
            Self::MissingField(field) => write!(formatter, "request is missing {field}"),
            Self::InvalidValue(field) => write!(formatter, "request field {field} is malformed"),
            Self::TrailingField => write!(formatter, "request has a trailing field"),
        }
    }
}

impl Error for ControlError {}
