//! VM-level state: certified memory-slot layout and fixed x86 VM addresses.

use super::{KvmStateError, invalid};
use crate::snapshot::wire::{Reader, Writer};

pub const MAX_MEMORY_SLOTS: u16 = 16;

/// One KVM user memory region backed by a range of the memory object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemorySlot {
    pub slot: u32,
    pub guest_address: u64,
    pub size: u64,
    pub memory_offset: u64,
}

impl MemorySlot {
    const ENCODED_LEN: usize = 4 + 8 + 8 + 8;

    const fn guest_end(&self) -> Option<u64> {
        self.guest_address.checked_add(self.size)
    }

    fn validate(&self) -> Result<(), KvmStateError> {
        if self.size == 0 {
            return Err(invalid("memory_slot.size", self.size));
        }
        if self.guest_end().is_none() {
            return Err(invalid("memory_slot.guest_address", self.guest_address));
        }
        if self.memory_offset.checked_add(self.size).is_none() {
            return Err(invalid("memory_slot.memory_offset", self.memory_offset));
        }
        Ok(())
    }

    fn write(&self, writer: &mut Writer) {
        writer.put_u32(self.slot);
        writer.put_u64(self.guest_address);
        writer.put_u64(self.size);
        writer.put_u64(self.memory_offset);
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, KvmStateError> {
        Ok(Self {
            slot: reader.u32()?,
            guest_address: reader.u64()?,
            size: reader.u64()?,
            memory_offset: reader.u64()?,
        })
    }
}

/// Complete `VmState` section payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmState {
    slots: Vec<MemorySlot>,
    tss_address: u64,
    identity_map_address: u64,
}

impl VmState {
    /// Requires slots in ascending guest order with unique slot numbers and no overlap.
    ///
    /// # Errors
    ///
    /// Returns [`KvmStateError`] for an empty, oversized, overflowing, duplicated, or
    /// overlapping layout.
    pub fn new(
        slots: Vec<MemorySlot>,
        tss_address: u64,
        identity_map_address: u64,
    ) -> Result<Self, KvmStateError> {
        if slots.is_empty() {
            return Err(invalid("memory_slots.count", 0_u8));
        }
        if slots.len() > usize::from(MAX_MEMORY_SLOTS) {
            return Err(KvmStateError::TooManyEntries {
                field: "memory_slots",
                count: slots.len(),
            });
        }
        for (position, slot) in slots.iter().enumerate() {
            slot.validate()?;
            if slots[..position].iter().any(|s| s.slot == slot.slot) {
                return Err(KvmStateError::DuplicateEntry {
                    field: "memory_slots",
                    key: u64::from(slot.slot),
                });
            }
            if let Some(previous) = position.checked_sub(1).map(|index| &slots[index])
                && previous
                    .guest_end()
                    .is_none_or(|end| end > slot.guest_address)
            {
                return Err(KvmStateError::Overlap {
                    field: "memory_slots",
                });
            }
        }
        Ok(Self {
            slots,
            tss_address,
            identity_map_address,
        })
    }

    #[must_use]
    pub fn slots(&self) -> &[MemorySlot] {
        &self.slots
    }

    #[must_use]
    pub const fn tss_address(&self) -> u64 {
        self.tss_address
    }

    #[must_use]
    pub const fn identity_map_address(&self) -> u64 {
        self.identity_map_address
    }

    /// Total guest RAM covered by all slots.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.slots
            .iter()
            .fold(0_u64, |total, slot| total.saturating_add(slot.size))
    }

    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::with_capacity(2 + self.slots.len() * MemorySlot::ENCODED_LEN + 16);
        writer.put_u16(u16::try_from(self.slots.len()).unwrap_or(MAX_MEMORY_SLOTS));
        for slot in &self.slots {
            slot.write(&mut writer);
        }
        writer.put_u64(self.tss_address);
        writer.put_u64(self.identity_map_address);
        writer.finish()
    }

    /// # Errors
    ///
    /// Returns [`KvmStateError`] for short, oversized, overlapping, or trailing input.
    pub fn decode(bytes: &[u8]) -> Result<Self, KvmStateError> {
        let mut reader = Reader::new(bytes);
        let count = reader.count_u16(MAX_MEMORY_SLOTS)?;
        let mut slots = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            slots.push(MemorySlot::read(&mut reader)?);
        }
        let tss_address = reader.u64()?;
        let identity_map_address = reader.u64()?;
        reader.finish()?;
        Self::new(slots, tss_address, identity_map_address)
    }
}
