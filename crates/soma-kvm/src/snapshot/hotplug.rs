//! The hotplug-region record: geometry only.
//!
//! A machine with a hotplug region captures only where the region is and how big it is. Which
//! blocks the guest had plugged is deliberately absent: a capture requires none plugged, and a
//! restore starts the region empty, so there is no state to carry. The section exists only for
//! such a machine; a machine without a region encodes exactly the sections it always did.

use std::{error::Error, fmt};

use super::{
    WireError,
    section::{Section, SectionError, SectionRole},
    wire::{Reader, Writer},
};
use crate::memory_layout::{
    HOTPLUG_BLOCK_BYTES, HOTPLUG_REGION_START, HotplugError, HotplugGeometry,
};

/// Version of the record encoding, carried inside the payload.
pub const RECORD_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotplugRecordError {
    Wire(WireError),
    UnsupportedVersion(u16),
    /// The region does not start where this implementation places it.
    GuestStart(u64),
    /// The block size is not the fixed 2 MiB.
    BlockBytes(u32),
    Geometry(HotplugError),
}

impl fmt::Display for HotplugRecordError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(error) => write!(formatter, "hotplug record wire error: {error}"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported hotplug record version {version}")
            }
            Self::GuestStart(start) => {
                write!(formatter, "hotplug region starts at unsupported {start:#x}")
            }
            Self::BlockBytes(bytes) => write!(formatter, "hotplug block size {bytes} is not fixed"),
            Self::Geometry(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for HotplugRecordError {}

impl From<WireError> for HotplugRecordError {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

/// Encodes the geometry of one region.
#[must_use]
pub fn encode(geometry: HotplugGeometry) -> Vec<u8> {
    let mut writer = Writer::with_capacity(22);
    writer.put_u16(RECORD_VERSION);
    writer.put_u64(geometry.guest_start());
    writer.put_u64(geometry.size());
    // The constant is 2 MiB, which fits `u32`.
    writer.put_u32(u32::try_from(HOTPLUG_BLOCK_BYTES).unwrap_or(u32::MAX));
    writer.finish()
}

/// Builds the manifest section for one region.
///
/// # Errors
///
/// Returns [`SectionError`] when the payload exceeds the section bound, which it cannot.
pub fn section(geometry: HotplugGeometry) -> Result<Section, SectionError> {
    Section::new(SectionRole::HotplugRegion, encode(geometry))
}

/// Decodes and validates one record.
///
/// # Errors
///
/// Returns a typed [`HotplugRecordError`] for a malformed, unsupported, or trailing record.
pub fn decode(bytes: &[u8]) -> Result<HotplugGeometry, HotplugRecordError> {
    let mut reader = Reader::new(bytes);
    let version = reader.u16()?;
    if version != RECORD_VERSION {
        return Err(HotplugRecordError::UnsupportedVersion(version));
    }
    let guest_start = reader.u64()?;
    let size = reader.u64()?;
    let block_bytes = reader.u32()?;
    reader.finish()?;
    if guest_start != HOTPLUG_REGION_START {
        return Err(HotplugRecordError::GuestStart(guest_start));
    }
    if u64::from(block_bytes) != HOTPLUG_BLOCK_BYTES {
        return Err(HotplugRecordError::BlockBytes(block_bytes));
    }
    HotplugGeometry::new(size).map_err(HotplugRecordError::Geometry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_layout::HOTPLUG_SLOT_BYTES;

    fn geometry() -> HotplugGeometry {
        HotplugGeometry::new(4 * HOTPLUG_SLOT_BYTES).unwrap()
    }

    #[test]
    fn round_trips_geometry_only() {
        let bytes = encode(geometry());
        // Geometry only: version, start, size, block size - no plugged-block state.
        assert_eq!(bytes.len(), 2 + 8 + 8 + 4);
        assert_eq!(decode(&bytes), Ok(geometry()));
    }

    #[test]
    fn rejects_every_other_shape() {
        let good = encode(geometry());
        let mut version = good.clone();
        version[1] = 2;
        assert_eq!(
            decode(&version),
            Err(HotplugRecordError::UnsupportedVersion(2))
        );

        let mut start = good.clone();
        start[9] ^= 1;
        assert!(matches!(
            decode(&start),
            Err(HotplugRecordError::GuestStart(_))
        ));

        let mut block = good.clone();
        block[19] ^= 1;
        assert!(matches!(
            decode(&block),
            Err(HotplugRecordError::BlockBytes(_))
        ));

        let mut size = good.clone();
        size[17] = 1;
        assert!(matches!(
            decode(&size),
            Err(HotplugRecordError::Geometry(_))
        ));

        let mut zero = good.clone();
        zero[10..18].fill(0);
        assert!(matches!(
            decode(&zero),
            Err(HotplugRecordError::Geometry(_))
        ));

        let mut trailing = good.clone();
        trailing.push(0);
        assert!(matches!(
            decode(&trailing),
            Err(HotplugRecordError::Wire(_))
        ));
        assert!(matches!(
            decode(&good[..good.len() - 1]),
            Err(HotplugRecordError::Wire(_))
        ));
    }

    #[test]
    fn the_section_sorts_between_the_devices_and_the_marker() {
        assert!(SectionRole::Device4.code() < SectionRole::HotplugRegion.code());
        assert!(SectionRole::HotplugRegion.code() < SectionRole::RepairPointMarker.code());
        assert!(!SectionRole::HotplugRegion.is_required());
        let section = section(geometry()).unwrap();
        assert_eq!(section.role(), SectionRole::HotplugRegion);
        assert_eq!(decode(section.payload()), Ok(geometry()));
    }
}
