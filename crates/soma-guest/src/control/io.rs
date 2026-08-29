use std::time::Instant;

/// Byte-oriented transport owned by one authenticated guest-control session.
///
/// Implementations must either fill the complete supplied read buffer or return an error.
/// They must either write the complete supplied byte slice or return an error.
/// Every operation must return no later than its absolute deadline, including when that deadline
/// has already elapsed.
/// Implementations own cancellation of any operating-system work needed to honor that contract.
pub trait ControlIo {
    /// Adapter-specific error that never crosses the session-owner interface.
    type Error;

    /// Reads exactly the supplied number of bytes.
    ///
    /// # Errors
    ///
    /// Returns the adapter error if the complete byte slice could not be read.
    fn read_exact(&mut self, bytes: &mut [u8], deadline: Instant) -> Result<(), Self::Error>;

    /// Writes the complete supplied byte slice.
    ///
    /// # Errors
    ///
    /// Returns the adapter error if the complete byte slice could not be written.
    fn write_all(&mut self, bytes: &[u8], deadline: Instant) -> Result<(), Self::Error>;

    /// Irreversibly closes or invalidates this transport without waiting for peer activity.
    ///
    /// Implementations must initiate cancellation and teardown locally and must not wait for a
    /// peer acknowledgement.
    fn poison(&mut self);
}

/// Host transport that can commit authenticated guest repair to the VMM.
pub trait HostControlIo: ControlIo {
    /// Commits the externally enforced repair gate after `RepairComplete` authenticates.
    ///
    /// # Errors
    ///
    /// Returns the adapter error if repair could not be committed atomically by the deadline.
    fn commit_repair(&mut self, deadline: Instant) -> Result<(), Self::Error>;
}

pub(crate) enum FrameReadError {
    Io,
    Length,
}

pub(crate) struct OwnedIo<I: ControlIo> {
    inner: I,
    poisoned: bool,
}

impl<I: ControlIo> OwnedIo<I> {
    pub(crate) const fn new(inner: I) -> Self {
        Self {
            inner,
            poisoned: false,
        }
    }

    pub(crate) fn read_frame(
        &mut self,
        maximum: usize,
        minimum: usize,
        deadline: Instant,
    ) -> Result<Vec<u8>, FrameReadError> {
        let mut header = [0_u8; 2];
        self.inner
            .read_exact(&mut header, deadline)
            .map_err(|_| FrameReadError::Io)?;
        let length = usize::from(u16::from_be_bytes(header));
        if length < minimum || length > maximum {
            return Err(FrameReadError::Length);
        }
        let mut frame = vec![0_u8; length + header.len()];
        frame[..2].copy_from_slice(&header);
        self.inner
            .read_exact(&mut frame[2..], deadline)
            .map_err(|_| FrameReadError::Io)?;
        Ok(frame)
    }

    pub(crate) fn write_all(&mut self, bytes: &[u8], deadline: Instant) -> Result<(), ()> {
        self.inner.write_all(bytes, deadline).map_err(|_| ())
    }

    pub(crate) fn poison_once(&mut self) {
        if !self.poisoned {
            self.poisoned = true;
            self.inner.poison();
        }
    }
}

impl<I: HostControlIo> OwnedIo<I> {
    pub(crate) fn commit_repair(&mut self, deadline: Instant) -> Result<(), ()> {
        self.inner.commit_repair(deadline).map_err(|_| ())
    }
}
