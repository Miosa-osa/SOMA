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
use crate::memory_layout::{HotplugError, HotplugRegion};

/// Version of the record encoding, carried inside the payload.
pub const RECORD_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotplugRecordError {
    Wire(WireError),
    UnsupportedVersion(u16),
    Geometry(HotplugError),
}

impl fmt::Display for HotplugRecordError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(error) => write!(formatter, "hotplug record wire error: {error}"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported hotplug record version {version}")
            }
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
pub fn encode(region: HotplugRegion) -> Vec<u8> {
    let mut writer = Writer::with_capacity(18);
    writer.put_u16(RECORD_VERSION);
    writer.put_u64(region.guest_start);
    writer.put_u64(region.size);
    writer.finish()
}

/// Builds the manifest section for one region.
///
/// # Errors
///
/// Returns [`SectionError`] when the payload exceeds the section bound, which it cannot.
pub fn section(region: HotplugRegion) -> Result<Section, SectionError> {
    Section::new(SectionRole::HotplugRegion, encode(region))
}

/// Decodes and validates one record.
///
/// # Errors
///
/// Returns a typed [`HotplugRecordError`] for a malformed, unsupported, or trailing record.
pub fn decode(bytes: &[u8]) -> Result<HotplugRegion, HotplugRecordError> {
    let mut reader = Reader::new(bytes);
    let version = reader.u16()?;
    if version != RECORD_VERSION {
        return Err(HotplugRecordError::UnsupportedVersion(version));
    }
    let guest_start = reader.u64()?;
    let size = reader.u64()?;
    reader.finish()?;
    HotplugRegion::from_parts(guest_start, size).map_err(HotplugRecordError::Geometry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_layout::{GuestLayout, HOTPLUG_ALIGN_BYTES, MIN_RAM_BYTES};

    fn region() -> HotplugRegion {
        HotplugRegion::new(
            GuestLayout::new(MIN_RAM_BYTES).unwrap(),
            4 * HOTPLUG_ALIGN_BYTES,
        )
        .unwrap()
    }

    #[test]
    fn round_trips_geometry_only() {
        let bytes = encode(region());
        // Geometry only: version, start, size - no plugged-block state.
        assert_eq!(bytes.len(), 2 + 8 + 8);
        assert_eq!(decode(&bytes), Ok(region()));
    }

    #[test]
    fn rejects_every_other_shape() {
        let good = encode(region());
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
            Err(HotplugRecordError::Geometry(_))
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
        let section = section(region()).unwrap();
        assert_eq!(section.role(), SectionRole::HotplugRegion);
        assert_eq!(decode(section.payload()), Ok(region()));
    }
}
