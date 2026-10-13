//! Why a data key could not be sealed, recorded, read, or destroyed.

use std::fmt;

/// Why a data key could not be sealed, recorded, read, or destroyed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyError {
    /// The host's master key could not be read or created; the value names the step.
    KeyFile(&'static str),
    /// The operating system refused to produce random bytes.
    Random,
    /// The cipher refused to seal the key.
    Seal,
    /// A sealed key could not be written or read under the durable record root.
    Record(&'static str),
    /// No sealed key exists for this volume.
    Missing,
}

impl fmt::Display for KeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KeyFile(step) => write!(formatter, "the host volume key: {step}"),
            Self::Random => write!(formatter, "the host could not produce random bytes"),
            Self::Seal => write!(formatter, "the data key could not be sealed"),
            Self::Record(step) => write!(formatter, "the sealed key: {step}"),
            Self::Missing => write!(formatter, "no sealed key exists for this volume"),
        }
    }
}
