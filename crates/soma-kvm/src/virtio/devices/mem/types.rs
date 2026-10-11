//! The small value types the memory device reports.

/// Saturating counters; never guest data.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MemCounters {
    pub plugged: u32,
    pub unplugged: u32,
    pub nacked: u32,
    pub errored: u32,
    pub rejected: u32,
    /// Times the host could not take a block back when the driver reset or a new one started.
    pub discard_failed: u32,
}

/// Why the host's request for a size was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SizeError {
    /// The size is not a whole number of blocks.
    Unaligned,
    /// The size exceeds the region.
    BeyondRegion,
}
