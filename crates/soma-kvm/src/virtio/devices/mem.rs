//! virtio-mem (device id 24): guest memory that the host grows and shrinks
//! one fixed-size block at a time through a single request queue.
//!
//! The device owns a block-aligned region of guest-physical address space.
//! The host sets `requested_size`; the guest driver plugs or unplugs blocks
//! to converge on it. Every request is validated against the geometry and
//! the plugged bitmap before the backend is asked to change the host mapping,
//! and the bitmap changes only after the backend succeeds.

pub mod backend;
pub mod bitmap;
pub mod request;
pub mod state;

use std::fmt;

use crate::virtio::device::{
    ActivateError, ConfigAccessError, DeviceStateError, VIRTIO_F_VERSION_1, VirtioDevice,
};
use crate::virtio::devices::segments::{read_readable, write_writable};
use crate::virtio::devices::service::{ChainHandler, DeviceFault};
use crate::virtio::guest_memory::GuestMemory;
use crate::virtio::queue::chain::{ChainLimits, DescriptorChain};
use backend::{MemBackend, MemBackendError};
use bitmap::{MAX_BLOCKS, PluggedBitmap, RangeState};
use request::{
    MemRequest, REQUEST_LEN, RESPONSE_LEN, RequestKind, VIRTIO_MEM_RESP_ACK, VIRTIO_MEM_RESP_BUSY,
    VIRTIO_MEM_RESP_ERROR, VIRTIO_MEM_RESP_NACK, VIRTIO_MEM_STATE_MIXED, VIRTIO_MEM_STATE_PLUGGED,
    VIRTIO_MEM_STATE_UNPLUGGED, encode_response,
};

/// Virtio device identifier for a memory device.
pub const VIRTIO_MEM_DEVICE_ID: u32 = 24;
/// Feature bit: unplugged memory is inaccessible to the guest.
pub const VIRTIO_MEM_F_UNPLUGGED_INACCESSIBLE: u64 = 1 << 1;
/// Exact feature allowlist.
pub const MEM_FEATURES: u64 = VIRTIO_F_VERSION_1 | VIRTIO_MEM_F_UNPLUGGED_INACCESSIBLE;
/// Queue count and maximum size: one request queue.
pub const MEM_QUEUE_MAX: [u16; 1] = [128];
/// Length of the device configuration space.
pub const MEM_CONFIG_LEN: usize = 56;
/// Smallest block size the device accepts.
pub const MIN_BLOCK_SIZE: u64 = 1 << 21;
/// Largest block size the device accepts.
pub const MAX_BLOCK_SIZE: u64 = 1 << 30;
/// Most blocks one plug, unplug, or state request may name.
pub const MAX_REQUEST_BLOCKS: u16 = 4096;
const CHAIN_LIMITS: ChainLimits = ChainLimits {
    max_descriptors: 8,
    max_bytes: 1 << 12,
};

const CFG_BLOCK_SIZE: usize = 0;
const CFG_NODE_ID: usize = 8;
const CFG_ADDR: usize = 16;
const CFG_REGION_SIZE: usize = 24;
const CFG_USABLE_REGION_SIZE: usize = 32;
const CFG_PLUGGED_SIZE: usize = 40;
const CFG_REQUESTED_SIZE: usize = 48;

/// A rejected device geometry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeometryError {
    /// The block size is not a power of two within the supported range.
    BadBlockSize,
    /// The region start is not block aligned or the region overflows.
    BadRegion,
    /// The region is empty, not whole blocks, or too many blocks.
    BadRegionSize,
}

impl fmt::Display for GeometryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "memory device geometry rejected: {self:?}")
    }
}

impl std::error::Error for GeometryError {}

/// A rejected requested size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestedSizeError {
    /// The size is not a whole number of blocks.
    Unaligned,
    /// The size exceeds the usable region.
    TooLarge,
}

impl fmt::Display for RequestedSizeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "requested size rejected: {self:?}")
    }
}

impl std::error::Error for RequestedSizeError {}

/// The validated, immutable shape of the device region.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemGeometry {
    block_size: u64,
    addr: u64,
    region_size: u64,
    node_id: u16,
}

impl MemGeometry {
    /// Validates a geometry.
    ///
    /// # Errors
    /// Rejects a block size that is not a power of two in
    /// [`MIN_BLOCK_SIZE`]..=[`MAX_BLOCK_SIZE`], an unaligned or overflowing
    /// region, and an empty, fractional, or oversized region.
    pub fn new(
        block_size: u64,
        addr: u64,
        region_size: u64,
        node_id: u16,
    ) -> Result<Self, GeometryError> {
        if !block_size.is_power_of_two() || !(MIN_BLOCK_SIZE..=MAX_BLOCK_SIZE).contains(&block_size)
        {
            return Err(GeometryError::BadBlockSize);
        }
        if !addr.is_multiple_of(block_size) || addr.checked_add(region_size).is_none() {
            return Err(GeometryError::BadRegion);
        }
        let blocks = region_size / block_size;
        if region_size == 0
            || !region_size.is_multiple_of(block_size)
            || usize::try_from(blocks).map_or(true, |blocks| blocks > MAX_BLOCKS)
        {
            return Err(GeometryError::BadRegionSize);
        }
        Ok(Self {
            block_size,
            addr,
            region_size,
            node_id,
        })
    }

    #[must_use]
    pub const fn block_size(&self) -> u64 {
        self.block_size
    }

    #[must_use]
    pub const fn addr(&self) -> u64 {
        self.addr
    }

    #[must_use]
    pub const fn region_size(&self) -> u64 {
        self.region_size
    }

    #[must_use]
    pub const fn node_id(&self) -> u16 {
        self.node_id
    }

    /// Number of blocks in the region.
    #[must_use]
    pub fn blocks(&self) -> usize {
        usize::try_from(self.region_size / self.block_size).unwrap_or(MAX_BLOCKS)
    }
}

/// Saturating counters; never guest data.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MemCounters {
    pub plug: u32,
    pub unplug: u32,
    pub unplug_all: u32,
    pub state: u32,
    pub nack: u32,
    pub busy: u32,
    pub error: u32,
    /// Chains too malformed to answer (wrong size, no room for a response).
    pub rejected: u32,
}

/// The memory device model.
pub struct MemDevice {
    geometry: MemGeometry,
    requested_size: u64,
    bitmap: PluggedBitmap,
    backend: Box<dyn MemBackend + Send>,
    activated: bool,
    counters: MemCounters,
}

impl MemDevice {
    /// Binds a geometry and a host backend; nothing is plugged and nothing
    /// is requested.
    ///
    /// # Panics
    /// Never: `MemGeometry` already bounds the block count.
    #[must_use]
    pub fn new(geometry: MemGeometry, backend: Box<dyn MemBackend + Send>) -> Self {
        Self {
            geometry,
            requested_size: 0,
            bitmap: PluggedBitmap::new(geometry.blocks()).expect("geometry bounds the block count"),
            backend,
            activated: false,
            counters: MemCounters::default(),
        }
    }

    #[must_use]
    pub const fn geometry(&self) -> MemGeometry {
        self.geometry
    }

    #[must_use]
    pub const fn counters(&self) -> MemCounters {
        self.counters
    }

    #[must_use]
    pub const fn is_activated(&self) -> bool {
        self.activated
    }

    #[must_use]
    pub const fn requested_size(&self) -> u64 {
        self.requested_size
    }

    /// Bytes currently plugged.
    #[must_use]
    pub fn plugged_size(&self) -> u64 {
        u64::try_from(self.bitmap.plugged_blocks())
            .unwrap_or(u64::MAX)
            .saturating_mul(self.geometry.block_size)
    }

    #[must_use]
    pub const fn bitmap(&self) -> &PluggedBitmap {
        &self.bitmap
    }

    /// Sets the size the guest driver should converge on.
    ///
    /// Returns whether the value changed, in which case the caller must
    /// signal a configuration change on the transport.
    ///
    /// # Errors
    /// Rejects a size that is not whole blocks or exceeds the region.
    pub const fn set_requested_size(&mut self, size: u64) -> Result<bool, RequestedSizeError> {
        if !size.is_multiple_of(self.geometry.block_size) {
            return Err(RequestedSizeError::Unaligned);
        }
        if size > self.geometry.region_size {
            return Err(RequestedSizeError::TooLarge);
        }
        let changed = size != self.requested_size;
        self.requested_size = size;
        Ok(changed)
    }

    fn config_bytes(&self) -> [u8; MEM_CONFIG_LEN] {
        let mut raw = [0u8; MEM_CONFIG_LEN];
        let mut put = |at: usize, value: u64| raw[at..at + 8].copy_from_slice(&value.to_le_bytes());
        put(CFG_BLOCK_SIZE, self.geometry.block_size);
        put(CFG_ADDR, self.geometry.addr);
        put(CFG_REGION_SIZE, self.geometry.region_size);
        put(CFG_USABLE_REGION_SIZE, self.geometry.region_size);
        put(CFG_PLUGGED_SIZE, self.plugged_size());
        put(CFG_REQUESTED_SIZE, self.requested_size);
        raw[CFG_NODE_ID..CFG_NODE_ID + 2].copy_from_slice(&self.geometry.node_id.to_le_bytes());
        raw
    }

    /// Resolves a request range to `(first block, count)` inside the region.
    fn block_range(&self, addr: u64, nb_blocks: u16) -> Option<(usize, usize)> {
        if nb_blocks == 0 || nb_blocks > MAX_REQUEST_BLOCKS {
            return None;
        }
        let offset = addr.checked_sub(self.geometry.addr)?;
        if offset % self.geometry.block_size != 0 {
            return None;
        }
        let first = usize::try_from(offset / self.geometry.block_size).ok()?;
        let count = usize::from(nb_blocks);
        (first.checked_add(count)? <= self.bitmap.blocks()).then_some((first, count))
    }

    /// Runs one request; returns the response type and state payload.
    fn execute(&mut self, request: &MemRequest) -> (u16, u16) {
        match request.kind {
            RequestKind::Plug => (self.plug(request), 0),
            RequestKind::Unplug => (self.unplug(request), 0),
            RequestKind::UnplugAll => (self.unplug_all(), 0),
            RequestKind::State => self.range_state(request),
        }
    }

    fn plug(&mut self, request: &MemRequest) -> u16 {
        self.counters.plug = self.counters.plug.saturating_add(1);
        let Some((first, count)) = self.block_range(request.addr, request.nb_blocks) else {
            return VIRTIO_MEM_RESP_ERROR;
        };
        match self.bitmap.range_state(first, count) {
            Ok(RangeState::Unplugged) => {}
            _ => return VIRTIO_MEM_RESP_ERROR,
        }
        let wanted = u64::try_from(count)
            .unwrap_or(u64::MAX)
            .saturating_mul(self.geometry.block_size);
        if self.plugged_size().saturating_add(wanted) > self.requested_size {
            return VIRTIO_MEM_RESP_NACK;
        }
        match self.backend.plug(request.addr, wanted) {
            Ok(()) => {}
            Err(MemBackendError::Busy) => return VIRTIO_MEM_RESP_BUSY,
            Err(MemBackendError::Failed) => return VIRTIO_MEM_RESP_ERROR,
        }
        match self.bitmap.plug(first, count) {
            Ok(()) => VIRTIO_MEM_RESP_ACK,
            Err(_) => VIRTIO_MEM_RESP_ERROR,
        }
    }

    fn unplug(&mut self, request: &MemRequest) -> u16 {
        self.counters.unplug = self.counters.unplug.saturating_add(1);
        let Some((first, count)) = self.block_range(request.addr, request.nb_blocks) else {
            return VIRTIO_MEM_RESP_ERROR;
        };
        match self.bitmap.range_state(first, count) {
            Ok(RangeState::Plugged) => {}
            _ => return VIRTIO_MEM_RESP_ERROR,
        }
        let len = u64::try_from(count)
            .unwrap_or(u64::MAX)
            .saturating_mul(self.geometry.block_size);
        match self.backend.unplug(request.addr, len) {
            Ok(()) => {}
            Err(MemBackendError::Busy) => return VIRTIO_MEM_RESP_BUSY,
            Err(MemBackendError::Failed) => return VIRTIO_MEM_RESP_ERROR,
        }
        match self.bitmap.unplug(first, count) {
            Ok(()) => VIRTIO_MEM_RESP_ACK,
            Err(_) => VIRTIO_MEM_RESP_ERROR,
        }
    }

    fn unplug_all(&mut self) -> u16 {
        self.counters.unplug_all = self.counters.unplug_all.saturating_add(1);
        match self.backend.unplug_all() {
            Ok(()) => {}
            Err(MemBackendError::Busy) => return VIRTIO_MEM_RESP_BUSY,
            Err(MemBackendError::Failed) => return VIRTIO_MEM_RESP_ERROR,
        }
        self.bitmap.clear();
        VIRTIO_MEM_RESP_ACK
    }

    fn range_state(&mut self, request: &MemRequest) -> (u16, u16) {
        self.counters.state = self.counters.state.saturating_add(1);
        let Some((first, count)) = self.block_range(request.addr, request.nb_blocks) else {
            return (VIRTIO_MEM_RESP_ERROR, 0);
        };
        match self.bitmap.range_state(first, count) {
            Ok(RangeState::Plugged) => (VIRTIO_MEM_RESP_ACK, VIRTIO_MEM_STATE_PLUGGED),
            Ok(RangeState::Unplugged) => (VIRTIO_MEM_RESP_ACK, VIRTIO_MEM_STATE_UNPLUGGED),
            Ok(RangeState::Mixed) => (VIRTIO_MEM_RESP_ACK, VIRTIO_MEM_STATE_MIXED),
            Err(_) => (VIRTIO_MEM_RESP_ERROR, 0),
        }
    }

    fn count_response(&mut self, kind: u16) {
        let slot = match kind {
            VIRTIO_MEM_RESP_NACK => &mut self.counters.nack,
            VIRTIO_MEM_RESP_BUSY => &mut self.counters.busy,
            VIRTIO_MEM_RESP_ERROR => &mut self.counters.error,
            _ => return,
        };
        *slot = slot.saturating_add(1);
    }
}

impl ChainHandler for MemDevice {
    fn chain_limits(&self, _queue: u16) -> ChainLimits {
        CHAIN_LIMITS
    }

    fn handle_chain<M: GuestMemory + ?Sized>(
        &mut self,
        _queue: u16,
        chain: &DescriptorChain,
        mem: &M,
    ) -> Result<u32, DeviceFault> {
        let room = chain.writable_len() >= RESPONSE_LEN as u64;
        if chain.readable_len() != REQUEST_LEN as u64 || !room {
            self.counters.rejected = self.counters.rejected.saturating_add(1);
            if room {
                self.count_response(VIRTIO_MEM_RESP_ERROR);
                return respond(mem, chain, VIRTIO_MEM_RESP_ERROR, 0);
            }
            return Ok(0);
        }
        let mut raw = [0u8; REQUEST_LEN];
        if read_readable(mem, chain, 0, &mut raw)? != REQUEST_LEN {
            return Err(DeviceFault::Protocol);
        }
        let Ok(request) = MemRequest::parse(&raw) else {
            self.counters.error = self.counters.error.saturating_add(1);
            return respond(mem, chain, VIRTIO_MEM_RESP_ERROR, 0);
        };
        let (kind, state) = self.execute(&request);
        self.count_response(kind);
        respond(mem, chain, kind, state)
    }
}

fn respond<M: GuestMemory + ?Sized>(
    mem: &M,
    chain: &DescriptorChain,
    kind: u16,
    state: u16,
) -> Result<u32, DeviceFault> {
    let raw = encode_response(kind, state);
    if write_writable(mem, chain, 0, &raw)? != RESPONSE_LEN {
        return Err(DeviceFault::Protocol);
    }
    u32::try_from(RESPONSE_LEN).map_err(|_| DeviceFault::Protocol)
}

impl VirtioDevice for MemDevice {
    fn device_id(&self) -> u32 {
        VIRTIO_MEM_DEVICE_ID
    }

    fn feature_allowlist(&self) -> u64 {
        MEM_FEATURES
    }

    fn queue_max_sizes(&self) -> &[u16] {
        &MEM_QUEUE_MAX
    }

    fn config_len(&self) -> usize {
        MEM_CONFIG_LEN
    }

    fn read_config(&self, offset: usize, buf: &mut [u8]) -> Result<(), ConfigAccessError> {
        let end = offset
            .checked_add(buf.len())
            .filter(|end| *end <= MEM_CONFIG_LEN)
            .ok_or(ConfigAccessError::OutOfBounds)?;
        buf.copy_from_slice(&self.config_bytes()[offset..end]);
        Ok(())
    }

    fn write_config(&mut self, offset: usize, data: &[u8]) -> Result<(), ConfigAccessError> {
        match offset.checked_add(data.len()) {
            Some(end) if end <= MEM_CONFIG_LEN => Err(ConfigAccessError::ReadOnly),
            _ => Err(ConfigAccessError::OutOfBounds),
        }
    }

    fn activate(&mut self, negotiated_features: u64) -> Result<(), ActivateError> {
        if negotiated_features & !MEM_FEATURES != 0 {
            return Err(ActivateError::UnsupportedFeatures {
                negotiated: negotiated_features,
            });
        }
        self.activated = true;
        Ok(())
    }

    /// Clears the activation only. Plugged blocks stay plugged across a
    /// device reset: the host mapping is the truth and a reset driver
    /// re-reads the config and, if it wants a clean slate, sends `UNPLUG_ALL`.
    fn reset(&mut self) {
        self.activated = false;
    }

    fn snapshot_state(&self) -> Vec<u8> {
        state::MemState::capture(self).to_bytes()
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), DeviceStateError> {
        state::MemState::from_bytes(bytes)?.apply(self)
    }
}

#[cfg(test)]
mod tests;
