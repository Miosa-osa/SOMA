//! Performing one portable filesystem operation on the thread that owns the session.
//!
//! The session is owned by the sandbox thread and is consumed and returned by every exchange, so
//! the chunk loop that moves a whole file has to run here rather than on the caller's side. That
//! is the only reason this is separate from the mapping in [`super::super::file`], which decides
//! what each operation means and holds no session at all.

use soma::{FileAnswer, FileRefusal};
use soma_guest::{GuestApi, RangeWrite, RepairedHostControl};

use super::super::file::{
    admissible, answer_from, needs_ranged_files, range_answer, read_answer, single_request,
    write_answer,
};
use super::super::identity::fresh16;
use super::super::io::HostIo;
use super::super::session::SessionError;

/// One repaired session and the answer it produced, or the failure that ended it.
type Answered<'a> = (RepairedHostControl<HostIo<'a>>, soma::FileAnswer);

/// Performs one operation and hands the session back for the next one.
///
/// `api` caches what the guest answered to the level probe, so a session probes at most once and
/// only when its first level 2 operation arrives. A guest that did not echo the nonce is treated
/// as level 1: the operation is refused as unsupported and nothing it would reject is sent.
pub(super) fn perform<'a>(
    repaired: RepairedHostControl<HostIo<'a>>,
    operation: &soma::FileOperation,
    bound: usize,
    api: &mut Option<GuestApi>,
) -> Result<Answered<'a>, SessionError> {
    if !admissible(operation) {
        return Ok((repaired, FileAnswer::Refused(FileRefusal::Failed)));
    }
    let repaired = if needs_ranged_files(operation) {
        let (repaired, level) = repaired
            .ensure_guest_api(api, || nonce_text(&fresh16()).into_bytes())
            .map_err(|_| SessionError::File)?;
        if !level.has_ranged_files() {
            return Ok((repaired, FileAnswer::Refused(FileRefusal::Unsupported)));
        }
        repaired
    } else {
        repaired
    };
    match operation {
        soma::FileOperation::ReadRange {
            path,
            offset,
            length,
        } => repaired
            .read_range(
                path,
                *offset,
                usize::try_from(*length).unwrap_or(usize::MAX),
                bound,
            )
            .map(|(session, read)| (session, range_answer(read)))
            .map_err(|_| SessionError::File),
        soma::FileOperation::WriteRange {
            path,
            offset,
            create,
            shorten,
            bytes,
        } => repaired
            .write_range(
                RangeWrite {
                    path,
                    offset: *offset,
                    create: *create,
                    shorten: *shorten,
                    bytes,
                },
                bound,
            )
            .map(|(session, written)| (session, write_answer(written, bytes.len())))
            .map_err(|_| SessionError::File),
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

/// Lowercase hexadecimal text of random bytes, which is alphanumeric as a probe nonce must be.
fn nonce_text(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}
