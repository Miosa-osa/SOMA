use std::{error::Error, fmt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationError {
    InvalidIdentity,
    InvalidMachineName,
    InvalidImageReference,
    InvalidDigest,
    InvalidPlatform,
    InvalidShape,
    InvalidNetworkPolicy,
    InvalidCommand,
    InvalidLimits,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidIdentity => "identity is outside the portable SOMA contract",
            Self::InvalidMachineName => "machine name is outside the portable SOMA contract",
            Self::InvalidImageReference => "OCI image reference is invalid",
            Self::InvalidDigest => "OCI digest is invalid",
            Self::InvalidPlatform => "OCI platform is invalid",
            Self::InvalidShape => "machine shape is invalid",
            Self::InvalidNetworkPolicy => "network policy is invalid",
            Self::InvalidCommand => "direct command is invalid",
            Self::InvalidLimits => "execution limits are invalid",
        };
        formatter.write_str(message)
    }
}

impl Error for ValidationError {}
