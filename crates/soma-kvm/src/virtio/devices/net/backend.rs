//! Network backends: a preopened TAP descriptor and an in-memory loopback.
//!
//! The device never opens `/dev/net/tun`; the privileged broker hands over an
//! already-configured nonblocking descriptor as an owned [`File`].

use std::collections::VecDeque;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd as _, RawFd};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Why a backend call failed; carries no descriptor or interface name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetBackendError {
    /// The host I/O failed with this kind.
    Io(io::ErrorKind),
    /// The host wrote fewer bytes than the frame.
    ShortWrite,
}

impl fmt::Display for NetBackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "network backend failed: {self:?}")
    }
}

impl std::error::Error for NetBackendError {}

/// One frame-oriented host endpoint.
pub trait NetBackend {
    /// Sends one complete validated frame.
    ///
    /// # Errors
    /// Returns the typed failure; the device drops the frame and counts it.
    fn transmit(&mut self, frame: &[u8]) -> Result<(), NetBackendError>;

    /// Reads one frame into `buf` without blocking; `None` when idle.
    ///
    /// # Errors
    /// Returns the typed failure; the device stops on a persistent failure.
    fn receive(&mut self, buf: &mut [u8]) -> Result<Option<usize>, NetBackendError>;

    /// The host descriptor that becomes readable when a frame arrives, when there is one.
    ///
    /// Nothing else tells the device thread that the host side has a frame: the guest only
    /// notifies the receive queue when it posts buffers, so a backend that cannot be watched
    /// is drained on those notifications alone. A backend fed entirely by the host, such as a
    /// TAP, must be watchable or its frames would wait for a guest notification that has no
    /// reason to come.
    ///
    /// The descriptor is borrowed for watching only; ownership stays with the backend.
    fn readable_fd(&self) -> Option<RawFd> {
        None
    }
}

/// Frame I/O on a preopened TAP descriptor.
///
/// The descriptor must already be nonblocking; a blocking descriptor would
/// stall the device thread on `receive`.
pub struct TapBackend {
    tap: File,
}

impl TapBackend {
    /// Takes ownership of the descriptor.
    #[must_use]
    pub const fn new(tap: File) -> Self {
        Self { tap }
    }
}

impl NetBackend for TapBackend {
    fn transmit(&mut self, frame: &[u8]) -> Result<(), NetBackendError> {
        let written = self
            .tap
            .write(frame)
            .map_err(|e| NetBackendError::Io(e.kind()))?;
        if written == frame.len() {
            Ok(())
        } else {
            Err(NetBackendError::ShortWrite)
        }
    }

    fn receive(&mut self, buf: &mut [u8]) -> Result<Option<usize>, NetBackendError> {
        match self.tap.read(buf) {
            Ok(0) => Ok(None),
            Ok(len) => Ok(Some(len)),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(NetBackendError::Io(error.kind())),
        }
    }

    fn readable_fd(&self) -> Option<RawFd> {
        Some(self.tap.as_raw_fd())
    }
}

/// Largest number of frames the loopback queues hold.
pub const LOOPBACK_QUEUE_LIMIT: usize = 64;

type FrameQueue = Arc<Mutex<VecDeque<Vec<u8>>>>;

fn frames(queue: &FrameQueue) -> MutexGuard<'_, VecDeque<Vec<u8>>> {
    // A poisoned queue only means a test thread panicked mid-push; the frames stay usable.
    queue.lock().unwrap_or_else(PoisonError::into_inner)
}

/// An in-memory backend with queues shared through [`LoopbackHandle`], so a
/// test can feed and observe frames while the device owns the backend.
///
/// It is also the link-down placeholder of the first sandbox slice: with the
/// host link gate down the device never calls it, and it is `Send` so the bus
/// that owns it can move to the device thread.
#[derive(Clone, Debug, Default)]
pub struct LoopbackBackend {
    sent: FrameQueue,
    inbound: FrameQueue,
    /// Also queue every transmitted frame for reception.
    pub echo: bool,
    /// When set, every operation fails with this kind.
    pub fail: Option<io::ErrorKind>,
}

/// The test's end of a [`LoopbackBackend`].
#[derive(Clone, Debug)]
pub struct LoopbackHandle {
    sent: FrameQueue,
    inbound: FrameQueue,
}

impl LoopbackBackend {
    /// Also queues transmitted frames for reception when `echo` is set.
    #[must_use]
    pub const fn with_echo(mut self, echo: bool) -> Self {
        self.echo = echo;
        self
    }

    /// Makes every operation fail with `kind`.
    #[must_use]
    pub const fn with_failure(mut self, kind: io::ErrorKind) -> Self {
        self.fail = Some(kind);
        self
    }

    /// The shared handle for feeding inbound and observing sent frames.
    #[must_use]
    pub fn handle(&self) -> LoopbackHandle {
        LoopbackHandle {
            sent: Arc::clone(&self.sent),
            inbound: Arc::clone(&self.inbound),
        }
    }
}

impl LoopbackHandle {
    /// Queues a frame for the guest to receive; drops when the queue is full.
    pub fn push_inbound(&self, frame: &[u8]) {
        let mut inbound = frames(&self.inbound);
        if inbound.len() < LOOPBACK_QUEUE_LIMIT {
            inbound.push_back(frame.to_vec());
        }
    }

    /// Removes and returns every transmitted frame so far.
    #[must_use]
    pub fn take_sent(&self) -> Vec<Vec<u8>> {
        frames(&self.sent).drain(..).collect()
    }
}

impl NetBackend for LoopbackBackend {
    fn transmit(&mut self, frame: &[u8]) -> Result<(), NetBackendError> {
        if let Some(kind) = self.fail {
            return Err(NetBackendError::Io(kind));
        }
        let mut sent = frames(&self.sent);
        if sent.len() < LOOPBACK_QUEUE_LIMIT {
            sent.push_back(frame.to_vec());
        }
        drop(sent);
        if self.echo {
            self.handle().push_inbound(frame);
        }
        Ok(())
    }

    fn receive(&mut self, buf: &mut [u8]) -> Result<Option<usize>, NetBackendError> {
        if let Some(kind) = self.fail {
            return Err(NetBackendError::Io(kind));
        }
        let Some(frame) = frames(&self.inbound).pop_front() else {
            return Ok(None);
        };
        let len = frame.len().min(buf.len());
        buf[..len].copy_from_slice(&frame[..len]);
        Ok(Some(len))
    }
}
