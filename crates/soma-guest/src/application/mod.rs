mod command;
mod filesystem;
mod frame;
mod guest;
mod host;
mod operation;
mod output;
mod pty;
mod terminal;

pub use command::{CommandContext, EnvironmentPair, GuestCommand};
pub(crate) use filesystem::check_path;
pub use filesystem::{
    DirectoryEntry, EntryKind, FileFailure, FileOutcome, FileRequest, MAX_CHUNK_BYTES, MAX_ENTRIES,
    MAX_FILE_MODE, MAX_PATH_BYTES,
};
pub use guest::GuestMessage;
pub use host::HostMessage;
pub use operation::OperationId;
pub use output::OutputChunk;
pub use pty::{
    MAX_PTY_CHUNK_BYTES, MAX_PTY_COLUMNS, MAX_PTY_ROWS, MAX_PTY_WAIT_MILLIS, PtyFailure,
    PtyOutcome, PtyRequest, PtySize,
};
pub use terminal::{TerminalReport, TerminalStatus};

use crate::MAX_RECORD_PAYLOAD;

pub(crate) const HEADER_SIZE: usize = 28;
pub(crate) const MAX_BODY_SIZE: usize = MAX_RECORD_PAYLOAD - HEADER_SIZE;
