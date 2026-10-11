//! The hotplug region: an anonymous, lazily populated mapping of its own behind one extra KVM
//! memory slot, whose 2 MiB blocks the guest plugs and unplugs at run time.
//!
//! The region is never part of boot RAM or the snapshot memory object. An unplugged block is
//! not addressable by any device and holds no host memory: unplugging discards its pages, so
//! the host gets the memory back at once and a later plug reads zero.

use std::{error::Error, fmt, sync::Mutex};

use kvm_ioctls::VmFd;

use super::{
    super::error::{MachineError, Phase},
    RamMapping,
};
use crate::memory_layout::{HOTPLUG_BLOCK_BYTES, HotplugError, HotplugGeometry};

/// Why a plug, unplug, or access was refused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HotplugRamError {
    /// The range is not a whole in-region block range.
    Range(HotplugError),
    /// Plug named a block that is already plugged.
    AlreadyPlugged { block: u64 },
    /// Unplug named a block that is not plugged.
    NotPlugged { block: u64 },
    /// The host refused to map or discard the memory.
    Host(MachineError),
}

impl fmt::Display for HotplugRamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Range(error) => write!(formatter, "{error}"),
            Self::AlreadyPlugged { block } => write!(formatter, "block {block} is already plugged"),
            Self::NotPlugged { block } => write!(formatter, "block {block} is not plugged"),
            Self::Host(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for HotplugRamError {}

impl From<HotplugError> for HotplugRamError {
    fn from(error: HotplugError) -> Self {
        Self::Range(error)
    }
}

/// One bit per block, and the running count, behind one lock so an access and an unplug never
/// interleave.
struct Blocks {
    bits: Vec<u64>,
    plugged: u64,
}

impl Blocks {
    fn get(&self, block: u64) -> bool {
        self.bits[word(block)] & bit(block) != 0
    }

    fn set(&mut self, block: u64, on: bool) {
        if on {
            self.bits[word(block)] |= bit(block);
        } else {
            self.bits[word(block)] &= !bit(block);
        }
    }
}

fn word(block: u64) -> usize {
    usize::try_from(block / 64).unwrap_or(usize::MAX)
}

const fn bit(block: u64) -> u64 {
    1 << (block % 64)
}

/// The hotplug region of one machine.
pub struct HotplugRam {
    mapping: RamMapping,
    geometry: HotplugGeometry,
    blocks: Mutex<Blocks>,
}

impl HotplugRam {
    /// Maps the region empty: no block is plugged and no host page is resident.
    pub(crate) fn map(geometry: HotplugGeometry) -> Result<Self, MachineError> {
        let length = usize::try_from(geometry.size())
            .map_err(|_| MachineError::invalid(Phase::MapMemory, "hotplug size exceeds usize"))?;
        let words = usize::try_from(geometry.block_count().div_ceil(64))
            .map_err(|_| MachineError::invalid(Phase::MapMemory, "hotplug blocks exceed usize"))?;
        Ok(Self {
            mapping: RamMapping::anonymous(length, Phase::MapMemory)?,
            geometry,
            blocks: Mutex::new(Blocks {
                bits: vec![0; words],
                plugged: 0,
            }),
        })
    }

    /// Registers the whole region as its own KVM memory slot.
    pub(crate) fn register(&self, vm: &VmFd) -> Result<(), MachineError> {
        self.mapping
            .register_range(vm, &self.geometry.region(), Phase::RegisterMemory)
    }

    /// The region's geometry, which is all a snapshot records about it.
    #[must_use]
    pub const fn geometry(&self) -> HotplugGeometry {
        self.geometry
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Blocks> {
        self.blocks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Bytes currently plugged.
    #[must_use]
    pub fn plugged_bytes(&self) -> u64 {
        self.lock().plugged * HOTPLUG_BLOCK_BYTES
    }

    /// Plugs every block of the guest range, all or none.
    ///
    /// # Errors
    ///
    /// Returns [`HotplugRamError`] for a bad range or a block that is already plugged.
    pub fn plug(&self, address: u64, length: u64) -> Result<(), HotplugRamError> {
        let (first, count) = self.geometry.blocks(address, length)?;
        let mut blocks = self.lock();
        if let Some(block) = (first..first + count).find(|block| blocks.get(*block)) {
            return Err(HotplugRamError::AlreadyPlugged { block });
        }
        for block in first..first + count {
            blocks.set(block, true);
        }
        blocks.plugged += count;
        Ok(())
    }

    /// Unplugs every block of the guest range, all or none, and returns its memory to the host.
    ///
    /// # Errors
    ///
    /// Returns [`HotplugRamError`] for a bad range, a block that is not plugged, or a host
    /// failure to discard; a failed discard leaves the range plugged.
    pub fn unplug(&self, address: u64, length: u64) -> Result<(), HotplugRamError> {
        let (first, count) = self.geometry.blocks(address, length)?;
        let mut blocks = self.lock();
        if let Some(block) = (first..first + count).find(|block| !blocks.get(*block)) {
            return Err(HotplugRamError::NotPlugged { block });
        }
        let offset = first * HOTPLUG_BLOCK_BYTES;
        let bytes = usize::try_from(count * HOTPLUG_BLOCK_BYTES).map_err(|_| {
            HotplugRamError::Host(MachineError::invalid(
                Phase::MapMemory,
                "unplug length exceeds usize",
            ))
        })?;
        self.mapping
            .discard(offset, bytes)
            .map_err(HotplugRamError::Host)?;
        for block in first..first + count {
            blocks.set(block, false);
        }
        blocks.plugged -= count;
        Ok(())
    }

    /// Whether every byte of `[address, address + length)` is in a plugged block.
    #[must_use]
    pub fn is_plugged(&self, address: u64, length: u64) -> bool {
        self.plugged_range(&self.lock(), address, length).is_some()
    }

    fn plugged_range(&self, blocks: &Blocks, address: u64, length: u64) -> Option<u64> {
        if length == 0 || !self.geometry.contains(address, length) {
            return None;
        }
        let offset = self.geometry.offset_of(address)?;
        let last = offset.checked_add(length - 1)? / HOTPLUG_BLOCK_BYTES;
        (offset / HOTPLUG_BLOCK_BYTES..=last)
            .all(|block| blocks.get(block))
            .then_some(offset)
    }

    /// Reads plugged bytes; false when any byte is outside a plugged block.
    pub(crate) fn read(&self, address: u64, buf: &mut [u8]) -> bool {
        let length = u64::try_from(buf.len()).unwrap_or(u64::MAX);
        let blocks = self.lock();
        self.plugged_range(&blocks, address, length)
            .is_some_and(|offset| self.mapping.read(offset, buf))
    }

    /// Writes plugged bytes; false when any byte is outside a plugged block.
    pub(crate) fn write(&self, address: u64, bytes: &[u8]) -> bool {
        let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let blocks = self.lock();
        self.plugged_range(&blocks, address, length)
            .is_some_and(|offset| self.mapping.write(offset, bytes))
    }
}

#[cfg(test)]
mod tests;
