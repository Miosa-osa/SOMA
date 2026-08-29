mod command;
mod frame;
mod guest;
mod host;
mod operation;
mod output;
mod terminal;

pub use command::GuestCommand;
pub use guest::GuestMessage;
pub use host::HostMessage;
pub use operation::OperationId;
pub use output::OutputChunk;
pub use terminal::{TerminalReport, TerminalStatus};

use crate::MAX_RECORD_PAYLOAD;

pub(crate) const HEADER_SIZE: usize = 28;
pub(crate) const MAX_BODY_SIZE: usize = MAX_RECORD_PAYLOAD - HEADER_SIZE;
