use core::fmt;

use crate::TerminalStatus;

/// Typed completion and exact authenticated output for one Execute operation.
#[derive(Eq, PartialEq)]
pub struct ExecuteOutcome {
    status: TerminalStatus,
    stdout: Box<[u8]>,
    stderr: Box<[u8]>,
}

impl fmt::Debug for ExecuteOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecuteOutcome")
            .field("status", &self.status)
            .field("stdout_bytes", &self.stdout.len())
            .field("stderr_bytes", &self.stderr.len())
            .finish()
    }
}

impl ExecuteOutcome {
    pub(crate) const fn new(status: TerminalStatus, stdout: Box<[u8]>, stderr: Box<[u8]>) -> Self {
        Self {
            status,
            stdout,
            stderr,
        }
    }

    /// Returns the valid terminal process or agent result.
    #[must_use]
    pub const fn status(&self) -> TerminalStatus {
        self.status
    }

    /// Returns exact authenticated stdout bytes.
    #[must_use]
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    /// Returns exact authenticated stderr bytes.
    #[must_use]
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }
}
