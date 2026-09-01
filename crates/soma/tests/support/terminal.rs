//! A terminal the engine's terminal path can be driven against without KVM.
//!
//! It is not an emulator. It holds the one session the protocol allows, remembers its dimensions,
//! and echoes what was written to it, which is enough to prove that the engine carries an open,
//! a write, a read, a resize and a close to a backend and carries each answer back bound to the
//! Instance it was asked about.

use std::sync::{Arc, Mutex};

use soma::{PtyAnswer, PtyOperation, PtyRefusal};

/// The one session a backend holds, shared with every clone of it.
pub type SharedTerminal = Arc<Mutex<Option<Session>>>;

pub struct Session {
    columns: u16,
    rows: u16,
    pending: Vec<u8>,
}

/// Answers one operation against the shared session.
pub fn answer_for(session: &mut Option<Session>, operation: &PtyOperation) -> PtyAnswer {
    match operation {
        PtyOperation::Open { columns, rows } => {
            if session.is_some() {
                return PtyAnswer::Refused(PtyRefusal::AlreadyOpen);
            }
            *session = Some(Session {
                columns: *columns,
                rows: *rows,
                pending: Vec::new(),
            });
            PtyAnswer::Opened {
                columns: *columns,
                rows: *rows,
            }
        }
        PtyOperation::Write { bytes } => match session {
            None => PtyAnswer::Refused(PtyRefusal::NoSession),
            Some(open) => {
                open.pending.extend_from_slice(bytes);
                PtyAnswer::Wrote {
                    bytes: u32::try_from(bytes.len()).unwrap_or(u32::MAX),
                }
            }
        },
        PtyOperation::Read { .. } => match session {
            None => PtyAnswer::Refused(PtyRefusal::NoSession),
            Some(open) => PtyAnswer::Output {
                bytes: std::mem::take(&mut open.pending),
                end: false,
            },
        },
        PtyOperation::Resize { columns, rows } => match session {
            None => PtyAnswer::Refused(PtyRefusal::NoSession),
            Some(open) => {
                open.columns = *columns;
                open.rows = *rows;
                PtyAnswer::Resized {
                    columns: open.columns,
                    rows: open.rows,
                }
            }
        },
        PtyOperation::Close => {
            if session.take().is_none() {
                return PtyAnswer::Refused(PtyRefusal::NoSession);
            }
            PtyAnswer::Closed
        }
    }
}
