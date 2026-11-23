//! Bounded typed sections: role, version, length, digest, critical flag, payload.
//!
//! The section sequence is canonical: roles are strictly ascending, so duplicates and
//! reordering are rejected by the same rule.
//! Unknown critical roles reject the whole snapshot; unknown non-critical roles are skipped
//! after their digest is verified.

use super::{
    Digest,
    wire::{Reader, Writer},
};

mod error;
mod frame;
pub use frame::Section;

pub use error::SectionError;

/// Version of every known section encoding in schema v1.
pub const SECTION_VERSION: u16 = 1;
/// Upper bound on one section payload.
pub const MAX_SECTION_BYTES: u32 = 1 << 20;
/// Upper bound on the number of sections in one manifest.
pub const MAX_SECTIONS: u16 = 32;
/// Encoded size of one section header.
pub const HEADER_LEN: usize = 2 + 2 + 1 + 4 + Digest::LEN;

const FLAG_CRITICAL: u8 = 0b0000_0001;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SectionRole {
    VmState,
    Vcpu0,
    Irqchip,
    IrqRouting,
    KvmClock,
    Pit,
    Vcpu1,
    Vcpu2,
    Vcpu3,
    Vcpu4,
    Vcpu5,
    Vcpu6,
    Vcpu7,
    Device0,
    Device1,
    Device2,
    Device3,
    Device4,
    Device5,
    Device6,
    /// Geometry of the optional hotplug region; present only on a machine that has one.
    HotplugRegion,
    RepairPointMarker,
    Vcpu8,
    Vcpu9,
    Vcpu10,
    Vcpu11,
    Vcpu12,
    Vcpu13,
    Vcpu14,
    Vcpu15,
}

impl SectionRole {
    pub const ALL: [Self; 30] = [
        Self::VmState,
        Self::Vcpu0,
        Self::Irqchip,
        Self::IrqRouting,
        Self::KvmClock,
        Self::Pit,
        Self::Vcpu1,
        Self::Vcpu2,
        Self::Vcpu3,
        Self::Vcpu4,
        Self::Vcpu5,
        Self::Vcpu6,
        Self::Vcpu7,
        Self::Device0,
        Self::Device1,
        Self::Device2,
        Self::Device3,
        Self::Device4,
        Self::Device5,
        Self::Device6,
        Self::HotplugRegion,
        Self::RepairPointMarker,
        Self::Vcpu8,
        Self::Vcpu9,
        Self::Vcpu10,
        Self::Vcpu11,
        Self::Vcpu12,
        Self::Vcpu13,
        Self::Vcpu14,
        Self::Vcpu15,
    ];

    /// The role of one vCPU's state section.
    ///
    /// The additional vCPUs use fresh section codes after the existing roles so older snapshots
    /// retain their original bytes and section ordering.
    #[must_use]
    pub const fn vcpu(index: u16) -> Option<Self> {
        match index {
            0 => Some(Self::Vcpu0),
            1 => Some(Self::Vcpu1),
            2 => Some(Self::Vcpu2),
            3 => Some(Self::Vcpu3),
            4 => Some(Self::Vcpu4),
            5 => Some(Self::Vcpu5),
            6 => Some(Self::Vcpu6),
            7 => Some(Self::Vcpu7),
            8 => Some(Self::Vcpu8),
            9 => Some(Self::Vcpu9),
            10 => Some(Self::Vcpu10),
            11 => Some(Self::Vcpu11),
            12 => Some(Self::Vcpu12),
            13 => Some(Self::Vcpu13),
            14 => Some(Self::Vcpu14),
            15 => Some(Self::Vcpu15),
            _ => None,
        }
    }

    /// The processor index this role carries, if it is a vCPU state section.
    #[must_use]
    pub const fn vcpu_index(self) -> Option<u16> {
        match self {
            Self::Vcpu0 => Some(0),
            Self::Vcpu1 => Some(1),
            Self::Vcpu2 => Some(2),
            Self::Vcpu3 => Some(3),
            Self::Vcpu4 => Some(4),
            Self::Vcpu5 => Some(5),
            Self::Vcpu6 => Some(6),
            Self::Vcpu7 => Some(7),
            Self::Vcpu8 => Some(8),
            Self::Vcpu9 => Some(9),
            Self::Vcpu10 => Some(10),
            Self::Vcpu11 => Some(11),
            Self::Vcpu12 => Some(12),
            Self::Vcpu13 => Some(13),
            Self::Vcpu14 => Some(14),
            Self::Vcpu15 => Some(15),
            _ => None,
        }
    }

    #[must_use]
    pub const fn code(self) -> u16 {
        match self {
            Self::VmState => 0x0001,
            Self::Vcpu0 => 0x0002,
            Self::Irqchip => 0x0003,
            Self::IrqRouting => 0x0004,
            Self::KvmClock => 0x0005,
            Self::Pit => 0x0006,
            Self::Vcpu1 => 0x0007,
            Self::Vcpu2 => 0x0008,
            Self::Vcpu3 => 0x0009,
            Self::Vcpu4 => 0x000a,
            Self::Vcpu5 => 0x000b,
            Self::Vcpu6 => 0x000c,
            Self::Vcpu7 => 0x000d,
            Self::Device0 => 0x0010,
            Self::Device1 => 0x0011,
            Self::Device2 => 0x0012,
            Self::Device3 => 0x0013,
            Self::Device4 => 0x0014,
            Self::Device5 => 0x0015,
            Self::Device6 => 0x0016,
            Self::HotplugRegion => 0x0018,
            Self::RepairPointMarker => 0x0020,
            Self::Vcpu8 => 0x0021,
            Self::Vcpu9 => 0x0022,
            Self::Vcpu10 => 0x0023,
            Self::Vcpu11 => 0x0024,
            Self::Vcpu12 => 0x0025,
            Self::Vcpu13 => 0x0026,
            Self::Vcpu14 => 0x0027,
            Self::Vcpu15 => 0x0028,
        }
    }

    #[must_use]
    pub fn from_code(code: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.code() == code)
    }

    /// Whether the codec refuses a manifest that lacks this section.
    ///
    /// The optional slots are structurally optional because a Generation that declared no
    /// writable storage, no network, no resizable memory or no durable volume never had those
    /// devices to capture. Which slots a manifest must carry is a statement about a particular
    /// Generation, so the compatibility check requires exactly its own sections.
    #[must_use]
    pub const fn is_required(self) -> bool {
        if let Some(index) = self.vcpu_index() {
            return index == 0;
        }
        !matches!(
            self,
            Self::Pit
                | Self::Device1
                | Self::Device2
                | Self::Device5
                | Self::Device6
                | Self::HotplugRegion
        )
    }

    /// The device slot carried by a device section.
    #[must_use]
    pub const fn device_slot(self) -> Option<u8> {
        match self {
            Self::Device0 => Some(0),
            Self::Device1 => Some(1),
            Self::Device2 => Some(2),
            Self::Device3 => Some(3),
            Self::Device4 => Some(4),
            Self::Device5 => Some(5),
            Self::Device6 => Some(6),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
