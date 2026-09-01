//! Performing one portable filesystem operation on the thread that owns the session.
//!
//! The session is owned by the sandbox thread and is consumed and returned by every exchange, so
//! the chunk loop that moves a whole file has to run here rather than on the caller's side. That
//! is the only reason this is separate from the mapping in [`super::super::file`], which decides
//! what each operation means and holds no session at all.

use soma_guest::RepairedHostControl;

use super::super::file::{answer_from, read_answer, single_request, write_answer};
use super::super::io::HostIo;
use super::super::session::SessionError;

/// One repaired session and the answer it produced, or the failure that ended it.
type Answered<'a> = (RepairedHostControl<HostIo<'a>>, soma::FileAnswer);

/// Performs one operation and hands the session back for the next one.
pub(super) fn perform<'a>(
    repaired: RepairedHostControl<HostIo<'a>>,
    operation: &soma::FileOperation,
    bound: usize,
) -> Result<Answered<'a>, SessionError> {
    match operation {
        soma::FileOperation::Read { path } => repaired
            .read_whole_file(path, bound)
            .map(|(session, read)| (session, read_answer(read)))
            .map_err(|_| SessionError::File),
        soma::FileOperation::Write { path, bytes } => repaired
            .write_whole_file(path, bytes, bound)
            .map(|(session, written)| (session, write_answer(written, bytes.len())))
            .map_err(|_| SessionError::File),
        other => {
            // The remaining four are exactly one guest request each, so the mapping builds it and
            // this only carries it. An operation the mapping does not build is a mapping bug
            // rather than a guest condition, and ends the session rather than inventing an answer.
            let request = single_request(other).ok_or(SessionError::File)?;
            repaired
                .file(request)
                .map(|(session, outcome)| (session, answer_from(outcome)))
                .map_err(|_| SessionError::File)
        }
    }
}
