//! One split virtqueue: geometry, ready state, cursors, and bounded ring access.
//!
//! Event-index notification suppression is not negotiated in v1, so only the
//! base `NO_INTERRUPT` flag is honored.

use std::sync::atomic::{Ordering, fence};

pub mod chain;
pub mod layout;
pub mod state;
pub mod violation;

use crate::virtio::guest_memory::{GuestAddress, GuestMemory, GuestMemoryError};
use chain::{ChainLimits, DescriptorChain, walk_chain};
use layout::{LayoutViolation, QueueLayout, validate_size};
use state::QueueState;
use violation::{QueueViolation, QueueViolationCounters};

/// Available-ring flag: the driver does not want a used-buffer interrupt.
pub const VIRTQ_AVAIL_F_NO_INTERRUPT: u16 = 1;

/// One split virtqueue owned by a transport.
#[derive(Clone, Debug)]
pub struct Queue {
    max_size: u16,
    size: u16,
    desc: GuestAddress,
    avail: GuestAddress,
    used: GuestAddress,
    layout: Option<QueueLayout>,
    activated: bool,
    next_avail: u16,
    next_used: u16,
    violations: QueueViolationCounters,
}

impl Queue {
    /// A reset queue with the given device maximum.
    ///
    /// # Errors
    /// Rejects a maximum that is zero, not a power of two, or above the ring limit.
    pub fn new(max_size: u16) -> Result<Self, LayoutViolation> {
        validate_size(max_size, chain::MAX_QUEUE_SIZE)?;
        Ok(Self {
            max_size,
            size: max_size,
            desc: GuestAddress(0),
            avail: GuestAddress(0),
            used: GuestAddress(0),
            layout: None,
            activated: false,
            next_avail: 0,
            next_used: 0,
            violations: QueueViolationCounters::default(),
        })
    }

    /// Device-fixed maximum size.
    #[must_use]
    pub const fn max_size(&self) -> u16 {
        self.max_size
    }

    /// Driver-selected size, defaulting to the maximum.
    #[must_use]
    pub const fn size(&self) -> u16 {
        self.size
    }

    /// Whether the queue is ready for work.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.layout.is_some()
    }

    /// Bounded violation counters for this queue.
    #[must_use]
    pub const fn violations(&self) -> &QueueViolationCounters {
        &self.violations
    }

    /// Stores a validated driver-selected size.
    ///
    /// # Errors
    /// Rejects invalid sizes without changing the queue.
    pub fn set_size(&mut self, size: u16) -> Result<(), QueueViolation> {
        self.size = validate_size(size, self.max_size)
            .map_err(|violation| self.record(QueueViolation::Layout(violation)))?;
        Ok(())
    }

    /// Stores the descriptor-table address; validated on activation.
    pub const fn set_desc_addr(&mut self, addr: u64) {
        self.desc = GuestAddress(addr);
    }

    /// Stores the available-ring address; validated on activation.
    pub const fn set_avail_addr(&mut self, addr: u64) {
        self.avail = GuestAddress(addr);
    }

    /// Stores the used-ring address; validated on activation.
    pub const fn set_used_addr(&mut self, addr: u64) {
        self.used = GuestAddress(addr);
    }

    /// Validates the geometry and marks the queue ready with zeroed cursors.
    ///
    /// # Errors
    /// Rejects a second activation before reset and any layout violation.
    pub fn activate<M: GuestMemory + ?Sized>(&mut self, mem: &M) -> Result<(), QueueViolation> {
        if self.activated {
            return Err(self.record(QueueViolation::AlreadyActivated));
        }
        let layout = QueueLayout::validate(
            mem,
            self.size,
            self.max_size,
            self.desc,
            self.avail,
            self.used,
        )
        .map_err(|violation| self.record(QueueViolation::Layout(violation)))?;
        self.layout = Some(layout);
        self.activated = true;
        self.next_avail = 0;
        self.next_used = 0;
        Ok(())
    }

    /// Stops accepting work without clearing configuration.
    pub const fn deactivate(&mut self) {
        self.layout = None;
    }

    /// Returns the queue to its post-reset state; counters are retained.
    pub const fn reset(&mut self) {
        self.size = self.max_size;
        self.desc = GuestAddress(0);
        self.avail = GuestAddress(0);
        self.used = GuestAddress(0);
        self.layout = None;
        self.activated = false;
        self.next_avail = 0;
        self.next_used = 0;
    }

    /// Number of heads the driver has published but the device has not popped.
    ///
    /// # Errors
    /// Rejects an unready queue, unreadable ring, or an index advanced past the size.
    pub fn pending<M: GuestMemory + ?Sized>(&mut self, mem: &M) -> Result<u16, QueueViolation> {
        let layout = self.ready_layout()?;
        let idx_addr = self.ring_addr(layout.avail(), 2)?;
        let avail_idx = mem
            .read_obj_at::<u16>(idx_addr)
            .map_err(|error| self.record(QueueViolation::Memory(error)))?;
        // The driver publishes descriptors before the index; pair its release.
        fence(Ordering::Acquire);
        let pending = avail_idx.wrapping_sub(self.next_avail);
        if pending > layout.size() {
            return Err(self.record(QueueViolation::AvailIndexOverrun {
                pending,
                size: layout.size(),
            }));
        }
        Ok(pending)
    }

    /// Pops and validates the next available chain, or `None` when idle.
    ///
    /// On a chain violation the head is consumed so the queue cannot spin on it;
    /// the caller decides whether to report it used with length zero.
    ///
    /// # Errors
    /// Returns the typed violation; the available cursor still advances for chain errors.
    pub fn pop_descriptor_chain<M: GuestMemory + ?Sized>(
        &mut self,
        mem: &M,
        limits: ChainLimits,
    ) -> Result<Option<DescriptorChain>, QueueViolation> {
        if self.pending(mem)? == 0 {
            return Ok(None);
        }
        let layout = self.ready_layout()?;
        let slot = u64::from(self.next_avail % layout.size());
        let head_addr = self.ring_addr(layout.avail(), 4 + 2 * slot)?;
        let head = mem
            .read_obj_at::<u16>(head_addr)
            .map_err(|error| self.record(QueueViolation::Memory(error)))?;
        self.next_avail = self.next_avail.wrapping_add(1);
        match walk_chain(mem, layout.desc(), layout.size(), head, limits) {
            Ok(chain) => Ok(Some(chain)),
            Err(violation) => Err(self.record(QueueViolation::Chain { head, violation })),
        }
    }

    /// Publishes `chain` as used with `len` written bytes.
    ///
    /// # Errors
    /// Rejects an unready queue, a length above the chain's writable capacity,
    /// or an unwritable ring.
    pub fn add_used<M: GuestMemory + ?Sized>(
        &mut self,
        mem: &M,
        chain: &DescriptorChain,
        len: u32,
    ) -> Result<(), QueueViolation> {
        let layout = self.ready_layout()?;
        if u64::from(len) > chain.writable_len() {
            return Err(self.record(QueueViolation::UsedLengthExceedsCapacity {
                len,
                capacity: chain.writable_len(),
            }));
        }
        let slot = u64::from(self.next_used % layout.size());
        let elem = self.ring_addr(layout.used(), 4 + 8 * slot)?;
        let idx_addr = self.ring_addr(layout.used(), 2)?;
        let mut raw = [0u8; 8];
        raw[0..4].copy_from_slice(&u32::from(chain.head()).to_le_bytes());
        raw[4..8].copy_from_slice(&len.to_le_bytes());
        mem.write_bytes(elem, &raw)
            .map_err(|error| self.record(QueueViolation::Memory(error)))?;
        // The element must be visible before the index that publishes it.
        fence(Ordering::Release);
        self.next_used = self.next_used.wrapping_add(1);
        mem.write_obj_at(idx_addr, self.next_used)
            .map_err(|error| self.record(QueueViolation::Memory(error)))?;
        Ok(())
    }

    /// Whether the driver wants an interrupt for newly used buffers.
    ///
    /// # Errors
    /// Rejects an unready queue or an unreadable ring.
    pub fn needs_notification<M: GuestMemory + ?Sized>(
        &mut self,
        mem: &M,
    ) -> Result<bool, QueueViolation> {
        let layout = self.ready_layout()?;
        let flags = mem
            .read_obj_at::<u16>(layout.avail())
            .map_err(|error| self.record(QueueViolation::Memory(error)))?;
        Ok(flags & VIRTQ_AVAIL_F_NO_INTERRUPT == 0)
    }

    /// Snapshot-visible state.
    #[must_use]
    pub const fn state(&self) -> QueueState {
        QueueState {
            size: self.size,
            ready: self.layout.is_some(),
            activated: self.activated,
            desc: self.desc.0,
            avail: self.avail.0,
            used: self.used.0,
            next_avail: self.next_avail,
            next_used: self.next_used,
        }
    }

    /// Ring offsets are inside a contained layout, so overflow is impossible;
    /// it is still reported as a typed violation rather than a panic.
    fn ring_addr(
        &mut self,
        base: GuestAddress,
        offset: u64,
    ) -> Result<GuestAddress, QueueViolation> {
        base.checked_add(offset).ok_or_else(|| {
            self.record(QueueViolation::Memory(GuestMemoryError::Overflow {
                addr: base,
                len: offset,
            }))
        })
    }

    fn ready_layout(&mut self) -> Result<QueueLayout, QueueViolation> {
        self.layout
            .ok_or_else(|| self.record(QueueViolation::NotReady))
    }

    fn record(&mut self, violation: QueueViolation) -> QueueViolation {
        self.violations.record(&violation);
        violation
    }
}

#[cfg(test)]
mod tests;
