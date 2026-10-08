//! One backed RAM range, one memory-map entry, and the kind of one mapped byte.
//!
//! These are pure data: the machine registers a range exactly as this describes it, and the
//! guest is told about a map entry exactly as this encodes it.

/// Whether one byte of the memory map is RAM the guest may use or address space it may not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryKind {
    /// Usable RAM.
    Ram,
    /// Reserved address space: the legacy hole or the MMIO hole.
    Reserved,
}

/// One entry of the PVH memory map.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryMapEntry {
    /// First guest-physical byte.
    pub address: u64,
    /// Length in bytes, never zero.
    pub size: u64,
    /// Whether the range is RAM or reserved.
    pub kind: MemoryKind,
}

/// One backed range of guest RAM and where its bytes live in the memory object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RamRegion {
    /// KVM memory slot number, ascending from zero.
    pub slot: u32,
    /// First guest-physical byte of the range.
    pub guest_start: u64,
    /// Offset of the range's first byte inside the memory object.
    pub host_offset: u64,
    /// Length in bytes, never zero.
    pub size: u64,
}

impl RamRegion {
    /// One past the last guest-physical byte.
    #[must_use]
    pub const fn guest_end(&self) -> u64 {
        self.guest_start + self.size
    }

    /// One past the last byte inside the memory object.
    #[must_use]
    pub const fn host_end(&self) -> u64 {
        self.host_offset + self.size
    }

    /// Whether `[address, address + length)` lies entirely inside this range.
    #[must_use]
    pub const fn contains(&self, address: u64, length: u64) -> bool {
        address >= self.guest_start && address + length <= self.guest_end()
    }
}
