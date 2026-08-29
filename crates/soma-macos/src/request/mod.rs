mod command;
mod identity;
mod limits;
mod network;
mod operation;

use std::{error::Error, fmt};

use serde::Serialize;

pub use command::GuestCommand;
pub use identity::{ImageReference, InstanceId};
pub(crate) use limits::MAX_OUTPUT_BYTES;
pub use limits::{ControlLimits, ExecutionLimits, MachineShape};
pub use network::{
    DnsConfiguration, NetworkConfiguration, NetworkPolicy, PublishedPort, TransportProtocol,
};
pub use operation::{CreateMachine, ExecuteCommand, OneShotRun, StopOptions};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct RequestError {
    field: &'static str,
    reason: RequestErrorReason,
}

impl RequestError {
    pub(super) const fn new(field: &'static str, reason: RequestErrorReason) -> Self {
        Self { field, reason }
    }

    #[must_use]
    pub const fn field(self) -> &'static str {
        self.field
    }

    #[must_use]
    pub const fn reason(self) -> RequestErrorReason {
        self.reason
    }
}

impl fmt::Display for RequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid {}: {}", self.field, self.reason)
    }
}

impl Error for RequestError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestErrorReason {
    Empty,
    Zero,
    TooLarge,
    InvalidIdentifier,
    InvalidCharacter,
    ContainsNul,
    NotAbsolute,
    NotMebibyteAligned,
}

impl fmt::Display for RequestErrorReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "cannot be empty",
            Self::Zero => "cannot be zero",
            Self::TooLarge => "exceeds the supported allowance",
            Self::InvalidIdentifier => "must be 32 lowercase hexadecimal characters",
            Self::InvalidCharacter => "contains a forbidden character",
            Self::ContainsNul => "contains NUL",
            Self::NotAbsolute => "must be an absolute guest path",
            Self::NotMebibyteAligned => "must be an exact MiB multiple",
        };
        formatter.write_str(message)
    }
}
