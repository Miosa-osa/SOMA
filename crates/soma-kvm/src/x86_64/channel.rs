//! The host end of the vsock control stream as a deadline-bounded byte channel.
//!
//! Reads drain the device's `HostEndpoint`, writes fill it and wake the device thread so the
//! bytes are delivered into posted receive buffers, and every wait is bounded by an absolute
//! deadline or ends early when the guest stops. The channel carries bytes only; the
//! authenticated protocol above it lives in `soma-guest`.

use std::{
    sync::{
        Arc, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use vmm_sys_util::eventfd::EventFd;

use super::devices::SharedBus;
use crate::virtio::HostEndpoint;

/// Why a channel operation stopped before completing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelError {
    /// The absolute deadline elapsed.
    Deadline,
    /// No open connection exists, or the guest closed it before the operation completed.
    Closed,
    /// The vCPU thread has stopped, so no guest can ever answer.
    GuestStopped,
}

impl std::fmt::Display for ChannelError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Deadline => "control channel deadline elapsed",
            Self::Closed => "control connection is closed",
            Self::GuestStopped => "guest vCPU has stopped",
        })
    }
}

impl std::error::Error for ChannelError {}

/// A cloneable handle on the single vsock control connection.
#[derive(Clone)]
pub struct ControlChannel {
    shared: Arc<SharedBus>,
    host_work: Arc<EventFd>,
    finished: Arc<AtomicBool>,
}

impl ControlChannel {
    pub(crate) fn new(
        shared: Arc<SharedBus>,
        host_work: Arc<EventFd>,
        finished: Arc<AtomicBool>,
    ) -> Self {
        Self {
            shared,
            host_work,
            finished,
        }
    }

    fn kick(&self) {
        let _ignored = self.host_work.write(1);
    }

    /// Runs `step` under the bus lock until it returns `Some`, waiting on the bus condition
    /// variable between attempts and stopping at the deadline or when the guest stops.
    fn until<T>(
        &self,
        deadline: Instant,
        mut step: impl FnMut(Option<&mut HostEndpoint>) -> Result<Option<T>, ChannelError>,
    ) -> Result<T, ChannelError> {
        let mut bus = self.shared.lock();
        loop {
            if self.finished.load(Ordering::Acquire) {
                return Err(ChannelError::GuestStopped);
            }
            if let Some(value) = step(bus.vsock_mut().device_mut().endpoint())? {
                return Ok(value);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ChannelError::Deadline);
            }
            bus = self
                .shared
                .changed()
                .wait_timeout(bus, remaining)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Waits until the guest agent's connection on the control port is open.
    ///
    /// # Errors
    ///
    /// Returns the deadline or guest-stopped failure.
    pub fn wait_connected(&self, deadline: Instant) -> Result<(), ChannelError> {
        self.until(deadline, |endpoint| {
            Ok(endpoint
                .is_some_and(|endpoint| endpoint.is_open())
                .then_some(()))
        })
    }

    /// Fills `buf` completely from the guest stream.
    ///
    /// # Errors
    ///
    /// Returns the typed failure; a partial read is never reported as success.
    pub fn read_exact(&self, buf: &mut [u8], deadline: Instant) -> Result<(), ChannelError> {
        let mut filled = 0;
        self.until(deadline, |endpoint| {
            let endpoint = endpoint.ok_or(ChannelError::Closed)?;
            let count = endpoint.read(&mut buf[filled..]);
            if count > 0 {
                filled += count;
                // Consumed bytes free host credit; the device thread sends the update.
                self.kick();
            }
            if filled == buf.len() {
                return Ok(Some(()));
            }
            if endpoint.at_eof() {
                return Err(ChannelError::Closed);
            }
            Ok(None)
        })
    }

    /// Queues all of `bytes` for the guest, waking the device thread for delivery.
    ///
    /// # Errors
    ///
    /// Returns the typed failure; a partial write is never reported as success.
    pub fn write_all(&self, bytes: &[u8], deadline: Instant) -> Result<(), ChannelError> {
        let mut written = 0;
        self.until(deadline, |endpoint| {
            let endpoint = endpoint.ok_or(ChannelError::Closed)?;
            if !endpoint.is_open() {
                return Err(ChannelError::Closed);
            }
            let count = endpoint.write(&bytes[written..]);
            if count > 0 {
                written += count;
                self.kick();
            }
            Ok((written == bytes.len()).then_some(()))
        })
    }

    /// Resets the connection locally; the device thread sends `RST` to the guest.
    pub fn poison(&self) {
        self.shared.lock().vsock_mut().device_mut().close_endpoint();
        self.kick();
        self.shared.notify_all();
    }
}
