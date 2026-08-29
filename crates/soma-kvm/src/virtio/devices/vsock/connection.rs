//! The host side of the single stream connection: bounded byte buffers in
//! both directions with credit accounting, exposed to the future control
//! owner as [`HostEndpoint`].

use std::collections::VecDeque;

use super::credit::{Credit, CreditError};
use super::packet::{VSOCK_SHUTDOWN_RCV, VSOCK_SHUTDOWN_SEND};

/// Largest number of host-written bytes waiting to be sent to the guest.
pub const HOST_TX_BUFFER: usize = 1 << 16;

/// Progress of a host-initiated orderly shutdown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostShutdown {
    None,
    Requested,
    Sent,
}

/// One accepted stream connection as seen by the host.
#[derive(Clone, Debug)]
pub struct HostEndpoint {
    peer_port: u32,
    generation: u32,
    credit: Credit,
    from_guest: VecDeque<u8>,
    to_guest: VecDeque<u8>,
    open: bool,
    /// Peer shutdown flags in protocol encoding (`RCV`, `SEND`).
    peer_shutdown: u32,
    host_shutdown: HostShutdown,
    credit_update_due: bool,
}

impl HostEndpoint {
    pub(super) fn accept(
        peer_port: u32,
        generation: u32,
        buf_alloc: u32,
        fwd_cnt: u32,
    ) -> Result<Self, CreditError> {
        Ok(Self {
            peer_port,
            generation,
            credit: Credit::new(buf_alloc, fwd_cnt)?,
            from_guest: VecDeque::new(),
            to_guest: VecDeque::new(),
            open: true,
            peer_shutdown: 0,
            host_shutdown: HostShutdown::None,
            credit_update_due: false,
        })
    }

    /// Copies buffered guest bytes into `buf`; returns the count, zero at
    /// end of stream or when nothing is buffered (see [`Self::at_eof`]).
    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        let mut count = 0;
        while count < buf.len() {
            let Some(byte) = self.from_guest.pop_front() else {
                break;
            };
            buf[count] = byte;
            count += 1;
        }
        if count > 0 {
            self.credit
                .consumed(u32::try_from(count).unwrap_or(u32::MAX));
            self.credit_update_due = true;
        }
        count
    }

    /// Queues host bytes for the guest; returns how many were accepted,
    /// bounded by [`HOST_TX_BUFFER`] and refused once the peer or host closed
    /// the send direction.
    pub fn write(&mut self, bytes: &[u8]) -> usize {
        if !self.open
            || self.peer_shutdown & VSOCK_SHUTDOWN_RCV != 0
            || self.host_shutdown != HostShutdown::None
        {
            return 0;
        }
        let room = HOST_TX_BUFFER.saturating_sub(self.to_guest.len());
        let take = bytes.len().min(room);
        self.to_guest.extend(&bytes[..take]);
        take
    }

    /// Requests an orderly shutdown of both directions; the device sends
    /// `SHUTDOWN` once queued data has drained.
    pub const fn shutdown(&mut self) {
        if matches!(self.host_shutdown, HostShutdown::None) {
            self.host_shutdown = HostShutdown::Requested;
        }
    }

    /// Whether the connection is still established.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.open
    }

    /// Whether the guest will send no more and everything buffered was read.
    #[must_use]
    pub fn at_eof(&self) -> bool {
        (self.peer_shutdown & VSOCK_SHUTDOWN_SEND != 0 || !self.open) && self.from_guest.is_empty()
    }

    /// Bytes readable right now.
    #[must_use]
    pub fn pending_read(&self) -> usize {
        self.from_guest.len()
    }

    /// Bytes written by the host and not yet sent.
    #[must_use]
    pub fn pending_write(&self) -> usize {
        self.to_guest.len()
    }

    #[must_use]
    pub const fn peer_port(&self) -> u32 {
        self.peer_port
    }

    /// Connection generation; bumps on every accept, reset, and restore.
    #[must_use]
    pub const fn generation(&self) -> u32 {
        self.generation
    }

    pub(super) fn update_peer_credit(
        &mut self,
        buf_alloc: u32,
        fwd_cnt: u32,
    ) -> Result<(), CreditError> {
        self.credit.update_peer(buf_alloc, fwd_cnt)
    }

    pub(super) fn push_from_guest(&mut self, bytes: &[u8]) -> Result<(), CreditError> {
        let len =
            u32::try_from(bytes.len()).map_err(|_| CreditError::Exceeded { len: u32::MAX })?;
        self.credit.accept_rx(len)?;
        self.from_guest.extend(bytes);
        Ok(())
    }

    /// Takes up to `max` bytes to send, bounded by peer credit.
    pub(super) fn take_to_guest(&mut self, max: usize) -> Vec<u8> {
        let free = usize::try_from(self.credit.peer_free()).unwrap_or(usize::MAX);
        let take = max.min(free).min(self.to_guest.len());
        let bytes: Vec<u8> = self.to_guest.drain(..take).collect();
        self.credit.sent(u32::try_from(take).unwrap_or(u32::MAX));
        self.credit_update_due = false;
        bytes
    }

    pub(super) fn can_send_data(&self) -> bool {
        self.open && !self.to_guest.is_empty() && self.credit.peer_free() > 0
    }

    pub(super) const fn local_credit(&self) -> (u32, u32) {
        self.credit.local_fields()
    }

    pub(super) const fn credit_update_due(&self) -> bool {
        self.credit_update_due
    }

    pub(super) const fn clear_credit_update(&mut self) {
        self.credit_update_due = false;
    }

    pub(super) const fn peer_shutdown(&mut self, flags: u32) {
        self.peer_shutdown |= flags & (VSOCK_SHUTDOWN_RCV | VSOCK_SHUTDOWN_SEND);
    }

    pub(super) const fn peer_fully_shut(&self) -> bool {
        self.peer_shutdown == VSOCK_SHUTDOWN_RCV | VSOCK_SHUTDOWN_SEND
    }

    /// Whether the device should send `SHUTDOWN` now: requested, not yet
    /// sent, and no host data waits.
    pub(super) fn shutdown_ready(&self) -> bool {
        self.open && self.host_shutdown == HostShutdown::Requested && self.to_guest.is_empty()
    }

    pub(super) const fn mark_shutdown_sent(&mut self) {
        self.host_shutdown = HostShutdown::Sent;
    }

    pub(super) fn close(&mut self) {
        self.open = false;
        self.to_guest.clear();
    }
}
