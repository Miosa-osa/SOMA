//! Device-specific snapshot fields from the device-surface contract.

use super::{DeviceKind, DeviceStateError};
use crate::snapshot::{
    Digest,
    wire::{Reader, Writer},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockState {
    pub capacity_sectors: u64,
    pub block_size: u32,
    /// Immutable root digest or sterile overlay-template digest.
    pub image_digest: Digest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceSpecific {
    Block(BlockState),
    Net {
        mac: [u8; 6],
        link_up: bool,
    },
    Vsock {
        cid_placeholder: u64,
    },
    Rng,
    /// The resizable region: where it starts and how large it is. Plugged memory is never part of
    /// a snapshot, so these two numbers are all the device has to say.
    Mem {
        addr: u64,
        region_size: u64,
    },
}

/// The unit the memory device plugs in, which the region must be a whole number of.
const MEM_BLOCK_SIZE: u64 = 2 * 1024 * 1024;

impl DeviceSpecific {
    /// # Errors
    ///
    /// Returns [`DeviceStateError::SpecificMismatch`] or [`DeviceStateError::InvalidField`].
    pub fn validate_for(&self, kind: DeviceKind) -> Result<(), DeviceStateError> {
        match (kind, self) {
            (
                DeviceKind::RootBlock | DeviceKind::OverlayBlock | DeviceKind::VolumeBlock,
                Self::Block(block),
            ) => block.validate(),
            (DeviceKind::Net, Self::Net { .. })
            | (DeviceKind::Vsock, Self::Vsock { .. })
            | (DeviceKind::Rng, Self::Rng) => Ok(()),
            (DeviceKind::Mem, Self::Mem { addr, region_size }) => {
                if *region_size == 0 || !region_size.is_multiple_of(MEM_BLOCK_SIZE) {
                    return Err(DeviceStateError::InvalidField {
                        field: "region_size",
                        value: *region_size,
                    });
                }
                if !addr.is_multiple_of(MEM_BLOCK_SIZE) || addr.checked_add(*region_size).is_none()
                {
                    return Err(DeviceStateError::InvalidField {
                        field: "region_addr",
                        value: *addr,
                    });
                }
                Ok(())
            }
            _ => Err(DeviceStateError::SpecificMismatch(kind)),
        }
    }

    pub(super) fn write(&self, writer: &mut Writer) {
        match self {
            Self::Block(block) => {
                writer.put_u64(block.capacity_sectors);
                writer.put_u32(block.block_size);
                writer.put_bytes(block.image_digest.as_bytes());
            }
            Self::Net { mac, link_up } => {
                writer.put_bytes(mac);
                writer.put_presence(*link_up);
            }
            Self::Vsock { cid_placeholder } => writer.put_u64(*cid_placeholder),
            Self::Rng => {}
            Self::Mem { addr, region_size } => {
                writer.put_u64(*addr);
                writer.put_u64(*region_size);
            }
        }
    }

    pub(super) fn read(
        reader: &mut Reader<'_>,
        kind: DeviceKind,
    ) -> Result<Self, DeviceStateError> {
        Ok(match kind {
            DeviceKind::RootBlock | DeviceKind::OverlayBlock | DeviceKind::VolumeBlock => {
                Self::Block(BlockState {
                    capacity_sectors: reader.u64()?,
                    block_size: reader.u32()?,
                    image_digest: Digest::from_bytes(reader.array()?),
                })
            }
            DeviceKind::Net => Self::Net {
                mac: reader.array()?,
                link_up: reader.presence()?,
            },
            DeviceKind::Vsock => Self::Vsock {
                cid_placeholder: reader.u64()?,
            },
            DeviceKind::Rng => Self::Rng,
            DeviceKind::Mem => Self::Mem {
                addr: reader.u64()?,
                region_size: reader.u64()?,
            },
        })
    }
}

impl BlockState {
    fn validate(&self) -> Result<(), DeviceStateError> {
        if !self.block_size.is_power_of_two() || self.block_size < 512 || self.block_size > 4096 {
            return Err(DeviceStateError::InvalidField {
                field: "block_size",
                value: u64::from(self.block_size),
            });
        }
        if self.capacity_sectors.checked_mul(512).is_none() {
            return Err(DeviceStateError::InvalidField {
                field: "capacity_sectors",
                value: self.capacity_sectors,
            });
        }
        if self.image_digest.is_zero() {
            return Err(DeviceStateError::InvalidField {
                field: "image_digest",
                value: 0,
            });
        }
        Ok(())
    }
}
