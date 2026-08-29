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
    Net { mac: [u8; 6], link_up: bool },
    Vsock { cid_placeholder: u64 },
    Rng,
}

impl DeviceSpecific {
    /// # Errors
    ///
    /// Returns [`DeviceStateError::SpecificMismatch`] or [`DeviceStateError::InvalidField`].
    pub fn validate_for(&self, kind: DeviceKind) -> Result<(), DeviceStateError> {
        match (kind, self) {
            (DeviceKind::RootBlock | DeviceKind::OverlayBlock, Self::Block(block)) => {
                block.validate()
            }
            (DeviceKind::Net, Self::Net { .. })
            | (DeviceKind::Vsock, Self::Vsock { .. })
            | (DeviceKind::Rng, Self::Rng) => Ok(()),
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
        }
    }

    pub(super) fn read(
        reader: &mut Reader<'_>,
        kind: DeviceKind,
    ) -> Result<Self, DeviceStateError> {
        Ok(match kind {
            DeviceKind::RootBlock | DeviceKind::OverlayBlock => Self::Block(BlockState {
                capacity_sectors: reader.u64()?,
                block_size: reader.u32()?,
                image_digest: Digest::from_bytes(reader.array()?),
            }),
            DeviceKind::Net => Self::Net {
                mac: reader.array()?,
                link_up: reader.presence()?,
            },
            DeviceKind::Vsock => Self::Vsock {
                cid_placeholder: reader.u64()?,
            },
            DeviceKind::Rng => Self::Rng,
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
