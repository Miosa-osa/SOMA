//! Performing one portable terminal operation on the thread that owns the session.
//!
//! The session is consumed and returned by every exchange, so this runs where the session lives
//! rather than on the caller's side. It is one exchange with no loop: a terminal call is one
//! guest record in each direction, which is what makes a terminal fit a request-and-answer
//! transport without a framing layer of its own.

use soma_guest::RepairedHostControl;

use super::super::io::HostIo;
use super::super::pty::{answer_from, guest_request};
use super::super::session::SessionError;

/// One repaired session and the answer it produced, or the failure that ended it.
type Answered<'a> = (RepairedHostControl<HostIo<'a>>, soma::PtyAnswer);

/// Performs one operation and hands the session back for the next one.
pub(super) fn perform<'a>(
    repaired: RepairedHostControl<HostIo<'a>>,
    operation: &soma::PtyOperation,
) -> Result<Answered<'a>, SessionError> {
    // A call the mapping will not build is a mapping defect rather than a guest condition, and
    // ends the session rather than inventing an answer for it.
    let request = guest_request(operation).ok_or(SessionError::Pty)?;
    repaired
        .pty(request)
        .map(|(session, outcome)| (session, answer_from(outcome)))
        .map_err(|_| SessionError::Pty)
}
