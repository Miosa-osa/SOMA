mod command;
mod filesystem;
mod frame;
mod guest;
mod host;
mod operation;
mod output;
mod pty;
mod terminal;

pub use command::{
    CommandContext, EnvironmentPair, FIXED_BODY_SIZE, GuestCommand, MAX_ARGUMENTS, MAX_FIELD_BYTES,
    MAX_TIMEOUT_MILLIS,
};
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
/// The largest body any one application message may carry inside one authenticated record.
///
/// This is the whole-message ceiling rather than a per-field one: every codec here bounds its own
/// fields as well, and the aggregate is what a single record can actually hold. It is public
/// because a host that decides whether a request is worth sending has to make that decision
/// against the same number this codec applies, and an aggregate a caller cannot see is one it
/// will rediscover as a failed message.
pub const MAX_BODY_SIZE: usize = MAX_RECORD_PAYLOAD - HEADER_SIZE;
