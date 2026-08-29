//! Checked conversions between SOMA state values and the `kvm-bindings` structs used by
//! `kvm-ioctls` on Linux `x86_64`.
//!
//! Nothing here touches a descriptor.
//! The later live slice reads KVM state into these structs, converts them into typed SOMA
//! values for encoding, and converts decoded values back before each `KVM_SET_*` ioctl.

mod clock;
#[allow(unsafe_code)]
mod irqchip;
#[allow(unsafe_code)]
mod nested;
mod regs;
mod tables;
mod vcpu;

use std::{error::Error, fmt};

use super::KvmStateError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingError {
    State(KvmStateError),
    FlagOutOfRange { field: &'static str, value: u8 },
    UnsupportedRouteType(u32),
    UnsupportedNestedFormat(u16),
    XsaveLength(usize),
    TableTooLarge { field: &'static str, count: usize },
}

impl fmt::Display for BindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::State(error) => write!(formatter, "{error}"),
            Self::FlagOutOfRange { field, value } => {
                write!(formatter, "KVM flag {field} must be 0 or 1, got {value}")
            }
            Self::UnsupportedRouteType(kind) => {
                write!(formatter, "unsupported KVM irq routing type {kind}")
            }
            Self::UnsupportedNestedFormat(format) => {
                write!(formatter, "unsupported KVM nested state format {format}")
            }
            Self::XsaveLength(length) => {
                write!(
                    formatter,
                    "XSAVE area of {length} bytes does not fit kvm_xsave"
                )
            }
            Self::TableTooLarge { field, count } => {
                write!(
                    formatter,
                    "{count} {field} entries exceed the KVM table bound"
                )
            }
        }
    }
}

impl Error for BindingError {}

impl From<KvmStateError> for BindingError {
    fn from(error: KvmStateError) -> Self {
        Self::State(error)
    }
}

pub(super) const fn flag(field: &'static str, value: u8) -> Result<bool, BindingError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(BindingError::FlagOutOfRange { field, value }),
    }
}
