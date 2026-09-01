//! Reading a completed command's output back through a packet channel.
//!
//! One `SOCK_SEQPACKET` datagram cannot carry the sixteen mebibytes an Execute is allowed to
//! produce, and a worker that answered with as much as fitted would be reporting a different
//! command's result than the one that ran. So the Executed receipt names the byte counts and
//! the supervisor reads the bytes back one bounded window at a time, out of the receipt the
//! worker already retains for exact replay. Nothing is recomputed and nothing is streamed:
//! reading the same window twice yields the same bytes.

use crate::OperationId;

/// The largest output window one reply packet carries.
///
/// The bytes travel hexadecimal, so a window costs twice its length on the wire. This is sized
/// so the whole maximum output of one command is fetched in a bounded number of exchanges over
/// a local socket rather than in thousands.
pub const MAX_OUTPUT_WINDOW_BYTES: u64 = 256 * 1024;

/// Which of a command's two output streams a window reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

impl OutputStream {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }

    #[must_use]
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "stdout" => Some(Self::Stdout),
            "stderr" => Some(Self::Stderr),
            _ => None,
        }
    }
}

/// One bounded window of one completed operation's output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutputWindow {
    operation_id: OperationId,
    stream: OutputStream,
    offset: u64,
    length: u64,
}

impl OutputWindow {
    /// Names one window, or `None` when it exceeds [`MAX_OUTPUT_WINDOW_BYTES`].
    ///
    /// A zero-length window is admitted: it is how a supervisor confirms an empty stream
    /// without a special case.
    #[must_use]
    pub const fn new(
        operation_id: OperationId,
        stream: OutputStream,
        offset: u64,
        length: u64,
    ) -> Option<Self> {
        if length > MAX_OUTPUT_WINDOW_BYTES {
            return None;
        }
        Some(Self {
            operation_id,
            stream,
            offset,
            length,
        })
    }

    #[must_use]
    pub const fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn stream(&self) -> OutputStream {
        self.stream
    }

    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    #[must_use]
    pub const fn length(&self) -> u64 {
        self.length
    }

    /// The window's byte range inside `total`, clamped to what exists.
    #[must_use]
    pub fn range(&self, total: usize) -> (usize, usize) {
        let start = usize::try_from(self.offset)
            .unwrap_or(usize::MAX)
            .min(total);
        let length = usize::try_from(self.length).unwrap_or(usize::MAX);
        (start, start.saturating_add(length).min(total))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operation() -> OperationId {
        OperationId::new([1; 16]).expect("operation")
    }

    #[test]
    fn a_window_larger_than_one_packet_is_refused() {
        assert!(
            OutputWindow::new(
                operation(),
                OutputStream::Stdout,
                0,
                MAX_OUTPUT_WINDOW_BYTES + 1
            )
            .is_none()
        );
        assert!(OutputWindow::new(operation(), OutputStream::Stderr, 0, 0).is_some());
    }

    #[test]
    fn a_window_never_reads_past_the_output_it_names() {
        let window = OutputWindow::new(operation(), OutputStream::Stdout, 6, 64).expect("window");
        assert_eq!(window.range(10), (6, 10));
        assert_eq!(window.range(4), (4, 4));
        assert_eq!(window.range(100), (6, 70));
    }

    #[test]
    fn stream_tokens_round_trip() {
        for stream in [OutputStream::Stdout, OutputStream::Stderr] {
            assert_eq!(OutputStream::from_token(stream.token()), Some(stream));
        }
        assert_eq!(OutputStream::from_token("serial"), None);
    }
}
