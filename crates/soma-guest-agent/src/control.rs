//! Deadline-aware byte transport over the fixed vsock control port.
//!
//! The guest connects from its assigned CID to `VMADDR_CID_HOST` on
//! [`CONTROL_VSOCK_PORT`]; the resulting stream implements the protocol crate's `ControlIo`
//! with absolute deadlines mapped onto socket timeouts.
//! The same adapter works over any stream with timeouts, so tests use a Unix socket pair.

#![allow(unsafe_code)]

use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::thread;
use std::time::{Duration, Instant};

use soma_guest::ControlIo;

use crate::ioctl;
use crate::timings::{self, Step};

pub use soma_guest::CONTROL_VSOCK_PORT;

const MIN_TIMEOUT: Duration = Duration::from_millis(1);
/// Interval between vsock context-identifier reads while waiting for a restored assignment.
const CID_POLL: Duration = Duration::from_millis(2);
const VSOCK_DEVICE: &str = "/dev/vsock";
const IOCTL_VM_SOCKETS_GET_LOCAL_CID: libc::c_ulong = 0x7b9;

/// Redacted transport failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IoFault {
    /// The absolute deadline elapsed before the operation completed.
    Expired,
    /// The socket failed or the peer closed it.
    Io,
    /// The transport was already poisoned.
    Poisoned,
}

/// Redacted vsock connection failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportError {
    /// The vsock device reported a CID other than the launch material's assignment.
    CidMismatch,
    /// The vsock device or socket could not be opened.
    Socket(i32),
    /// The host control endpoint refused the connection.
    Connect(i32),
}

/// A stream whose blocking operations accept a bounded timeout.
pub trait DeadlineStream: Read + Write {
    /// Bounds the next read.
    fn set_read_timeout(&mut self, timeout: Duration) -> io::Result<()>;
    /// Bounds the next write.
    fn set_write_timeout(&mut self, timeout: Duration) -> io::Result<()>;
    /// Closes both directions without waiting for the peer.
    fn close(&mut self);
}

impl DeadlineStream for UnixStream {
    fn set_read_timeout(&mut self, timeout: Duration) -> io::Result<()> {
        UnixStream::set_read_timeout(self, Some(timeout))
    }

    fn set_write_timeout(&mut self, timeout: Duration) -> io::Result<()> {
        UnixStream::set_write_timeout(self, Some(timeout))
    }

    fn close(&mut self) {
        let _ = self.shutdown(Shutdown::Both);
    }
}

/// `ControlIo` adapter that turns absolute deadlines into bounded stream operations.
#[derive(Debug)]
pub struct StreamIo<S: DeadlineStream> {
    stream: S,
    poisoned: bool,
}

impl<S: DeadlineStream> StreamIo<S> {
    /// Wraps a connected stream.
    pub const fn new(stream: S) -> Self {
        Self {
            stream,
            poisoned: false,
        }
    }

    fn remaining(&self, deadline: Instant) -> Result<Duration, IoFault> {
        if self.poisoned {
            return Err(IoFault::Poisoned);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(IoFault::Expired);
        }
        Ok(remaining.max(MIN_TIMEOUT))
    }
}

impl<S: DeadlineStream> ControlIo for StreamIo<S> {
    type Error = IoFault;

    fn read_exact(&mut self, bytes: &mut [u8], deadline: Instant) -> Result<(), IoFault> {
        timings::transport_read(|| self.fill(bytes, deadline))
    }

    fn write_all(&mut self, bytes: &[u8], deadline: Instant) -> Result<(), IoFault> {
        timings::transport_write(|| self.drain(bytes, deadline))
    }

    fn poison(&mut self) {
        self.poisoned = true;
        self.stream.close();
    }
}

impl<S: DeadlineStream> StreamIo<S> {
    /// Fills the whole slice, recording nothing so the caller owns the accounting.
    fn fill(&mut self, bytes: &mut [u8], deadline: Instant) -> Result<(), IoFault> {
        let mut filled = 0;
        while filled < bytes.len() {
            let remaining = self.remaining(deadline)?;
            self.stream
                .set_read_timeout(remaining)
                .map_err(|_| IoFault::Io)?;
            match self.stream.read(&mut bytes[filled..]) {
                Ok(0) => return Err(IoFault::Io),
                Ok(count) => filled += count,
                Err(error) if is_timeout(&error) => return Err(IoFault::Expired),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return Err(IoFault::Io),
            }
        }
        Ok(())
    }

    /// Writes the whole slice, recording nothing so the caller owns the accounting.
    fn drain(&mut self, bytes: &[u8], deadline: Instant) -> Result<(), IoFault> {
        let mut written = 0;
        while written < bytes.len() {
            let remaining = self.remaining(deadline)?;
            self.stream
                .set_write_timeout(remaining)
                .map_err(|_| IoFault::Io)?;
            match self.stream.write(&bytes[written..]) {
                Ok(0) => return Err(IoFault::Io),
                Ok(count) => written += count,
                Err(error) if is_timeout(&error) => return Err(IoFault::Expired),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return Err(IoFault::Io),
            }
        }
        Ok(())
    }
}

fn is_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// Verifies the assigned CID and connects to the host control endpoint before the deadline.
///
/// # Errors
///
/// Returns a redacted device, socket, or connection failure.
pub fn connect_vsock(
    expected_cid: u32,
    deadline: Instant,
) -> Result<StreamIo<UnixStream>, TransportError> {
    timings::measure(Step::CidWait, || await_local_cid(expected_cid, deadline))?;
    let opened = Instant::now();
    // SAFETY: `socket` has no memory preconditions; the descriptor is checked before use.
    let fd = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(TransportError::Socket(last_errno()));
    }
    // SAFETY: `fd` is a fresh descriptor owned by nothing else.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    let family = libc::sa_family_t::try_from(libc::AF_VSOCK)
        .map_err(|_| TransportError::Socket(libc::EINVAL))?;
    let address = libc::sockaddr_vm {
        svm_family: family,
        svm_reserved1: 0,
        svm_port: CONTROL_VSOCK_PORT,
        svm_cid: libc::VMADDR_CID_HOST,
        svm_zero: [0; 4],
    };
    let length = libc::socklen_t::try_from(std::mem::size_of::<libc::sockaddr_vm>())
        .map_err(|_| TransportError::Socket(libc::EINVAL))?;
    let timeout = deadline
        .saturating_duration_since(Instant::now())
        .max(MIN_TIMEOUT);
    let stream = UnixStream::from(owned);
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| TransportError::Socket(errno_of(&error)))?;
    // SAFETY: `connect` reads exactly `length` bytes from the valid `sockaddr_vm` local.
    let connected = unsafe {
        libc::connect(
            stream.as_raw_fd(),
            (&raw const address).cast::<libc::sockaddr>(),
            length,
        )
    };
    if connected != 0 {
        return Err(TransportError::Connect(last_errno()));
    }
    timings::record(Step::VsockConnect, opened.elapsed());
    Ok(StreamIo::new(stream))
}

/// Waits until the vsock device reports exactly the assigned CID, or the deadline passes.
///
/// On a cold boot the driver already read the assigned CID from configuration space, so the
/// first attempt succeeds. After a snapshot restore the VMM installs a fresh CID and queues
/// the transport-reset event that makes the driver re-read it, so the guest may briefly still
/// report the captured CID; polling turns that ordering into a bounded wait instead of a
/// spurious mismatch, and a genuinely wrong CID still fails closed at the deadline.
fn await_local_cid(expected: u32, deadline: Instant) -> Result<(), TransportError> {
    loop {
        let actual = read_local_cid()?;
        if actual == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(TransportError::CidMismatch);
        }
        thread::sleep(CID_POLL);
    }
}

fn read_local_cid() -> Result<u32, TransportError> {
    let device = std::fs::File::open(VSOCK_DEVICE)
        .map_err(|error| TransportError::Socket(errno_of(&error)))?;
    let mut cid: u32 = 0;
    // SAFETY: `IOCTL_VM_SOCKETS_GET_LOCAL_CID` writes one `u32` into the valid local.
    let result = unsafe {
        libc::ioctl(
            device.as_raw_fd(),
            ioctl::request(IOCTL_VM_SOCKETS_GET_LOCAL_CID),
            &raw mut cid,
        )
    };
    if result != 0 {
        return Err(TransportError::Socket(last_errno()));
    }
    Ok(cid)
}

fn errno_of(error: &io::Error) -> i32 {
    error.raw_os_error().unwrap_or(0)
}

fn last_errno() -> i32 {
    errno_of(&io::Error::last_os_error())
}

#[cfg(test)]
mod tests;
