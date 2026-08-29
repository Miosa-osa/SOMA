//! Typed rejection reasons; each names the exact field and both values.

use std::{error::Error, fmt};

use crate::snapshot::{
    Digest,
    device_state::DeviceStateError,
    kvm_state::KvmStateError,
    manifest::{Architecture, HostCapability},
    section::SectionRole,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Incompatibility {
    SchemaVersion {
        expected: u16,
        actual: u16,
    },
    Architecture {
        expected: Architecture,
        actual: Architecture,
    },
    PageSize {
        expected: u32,
        actual: u32,
    },
    MemoryLayout {
        expected: u64,
        actual: u64,
    },
    VcpuCount {
        expected: u16,
        actual: u16,
    },
    CpuTemplate {
        expected: Digest,
        actual: Digest,
    },
    KvmApiVersion {
        expected: u32,
        actual: u32,
    },
    MissingCapability(HostCapability),
    MemorySlots {
        required: u16,
        available: u16,
    },
    MachineContract {
        expected: Digest,
        actual: Digest,
    },
    DeviceContract {
        expected: Digest,
        actual: Digest,
    },
    GuestProtocolVersion {
        expected: u16,
        actual: u16,
    },
    MalformedVmState(KvmStateError),
    MissingSection(SectionRole),
    NoExpectationForSlot(u8),
    MalformedDevice {
        slot: u8,
        error: DeviceStateError,
    },
    QueueLimit {
        slot: u8,
        queue: u8,
        expected: u16,
        actual: u16,
    },
    FeatureNegotiation {
        slot: u8,
        expected: u64,
        actual: u64,
    },
}

impl fmt::Display for Incompatibility {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "snapshot incompatible with host: {self:?}")
    }
}

impl Error for Incompatibility {}
