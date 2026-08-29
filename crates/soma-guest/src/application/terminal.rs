use crate::Error;

use super::{command::MAX_OUTPUT_BYTES, frame::Reader};

const BODY_SIZE: usize = 16;

/// The final Linux process or agent outcome for one command operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalStatus {
    /// The process exited normally with this status code.
    Exited(i32),
    /// The process was terminated by this Linux signal.
    Signaled(u8),
    /// The command deadline expired.
    TimedOut,
    /// The combined output allowance was exhausted.
    OutputLimit,
    /// `execve` failed with this positive Linux errno.
    ExecFailed(i32),
    /// The trusted guest agent failed with this positive internal code.
    AgentFailed(i32),
}

/// A terminal status bound to the exact authenticated output byte counts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalReport {
    status: TerminalStatus,
    stdout_bytes: u32,
    stderr_bytes: u32,
}

impl TerminalStatus {
    fn fields(self) -> Result<(u8, i32), Error> {
        let fields = match self {
            Self::Exited(code @ 0..=255) => (1, code),
            Self::Signaled(signal @ 1..=64) => (2, i32::from(signal)),
            Self::TimedOut => (3, 0),
            Self::OutputLimit => (4, 0),
            Self::ExecFailed(errno @ 1..=4095) => (5, errno),
            Self::AgentFailed(code @ 1..=4095) => (6, code),
            _ => return Err(Error::InvalidTerminalStatus),
        };
        Ok(fields)
    }

    fn from_fields(kind: u8, detail: i32) -> Result<Self, Error> {
        match (kind, detail) {
            (1, code @ 0..=255) => Ok(Self::Exited(code)),
            (2, signal @ 1..=64) => Ok(Self::Signaled(
                u8::try_from(signal).map_err(|_| Error::ApplicationMessageRejected)?,
            )),
            (3, 0) => Ok(Self::TimedOut),
            (4, 0) => Ok(Self::OutputLimit),
            (5, errno @ 1..=4095) => Ok(Self::ExecFailed(errno)),
            (6, code @ 1..=4095) => Ok(Self::AgentFailed(code)),
            _ => Err(Error::ApplicationMessageRejected),
        }
    }
}

impl TerminalReport {
    /// Creates a terminal report with exact stdout and stderr byte counts.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid status or counts beyond the protocol output maximum.
    pub fn new(
        status: TerminalStatus,
        stdout_bytes: u32,
        stderr_bytes: u32,
    ) -> Result<Self, Error> {
        status.fields()?;
        let total = u64::from(stdout_bytes)
            .checked_add(u64::from(stderr_bytes))
            .ok_or(Error::InvalidTerminalReport)?;
        if total > MAX_OUTPUT_BYTES {
            return Err(Error::InvalidTerminalReport);
        }
        Ok(Self {
            status,
            stdout_bytes,
            stderr_bytes,
        })
    }

    /// Returns the exact typed terminal status.
    #[must_use]
    pub const fn status(self) -> TerminalStatus {
        self.status
    }

    /// Returns stdout bytes observed by the guest agent.
    #[must_use]
    pub const fn stdout_bytes(self) -> u32 {
        self.stdout_bytes
    }

    /// Returns stderr bytes observed by the guest agent.
    #[must_use]
    pub const fn stderr_bytes(self) -> u32 {
        self.stderr_bytes
    }

    pub(super) fn encode(self) -> Result<[u8; BODY_SIZE], Error> {
        let (kind, detail) = self.status.fields()?;
        let mut encoded = [0; BODY_SIZE];
        encoded[0] = kind;
        encoded[4..8].copy_from_slice(&detail.to_be_bytes());
        encoded[8..12].copy_from_slice(&self.stdout_bytes.to_be_bytes());
        encoded[12..16].copy_from_slice(&self.stderr_bytes.to_be_bytes());
        Ok(encoded)
    }

    pub(super) fn decode(body: &[u8]) -> Result<Self, Error> {
        let mut reader = Reader::new(body);
        let kind = reader.u8()?;
        if reader.take(3)?.iter().any(|byte| *byte != 0) {
            return Err(Error::ApplicationMessageRejected);
        }
        let detail = reader.i32()?;
        let stdout_bytes = reader.u32()?;
        let stderr_bytes = reader.u32()?;
        reader.finish()?;
        let status = TerminalStatus::from_fields(kind, detail)?;
        Self::new(status, stdout_bytes, stderr_bytes).map_err(|_| Error::ApplicationMessageRejected)
    }
}
