//! Placing this Instance's secrets before the sandbox is announced Ready.
//!
//! The placement sits between the repaired session and Ready for two reasons. It is the first
//! moment a secret can be delivered at all, because before it there is no authenticated session
//! to deliver over; and it is the last moment a failure is still free, because nothing outside
//! this thread has been told the sandbox exists.
//!
//! A launch that cannot place a secret therefore ends here. The error travels out of the sandbox
//! thread, which finishes the machine on its way out, so a caller never receives a sandbox that
//! is running without the credential it was launched with. The private overlay head goes with
//! the machine, so a partly written destination does not outlive the failure either.

use soma_guest::{RepairedHostControl, SecretFile, SecretPlacement};

use super::io::HostIo;
use super::session::SessionError;

/// Places every secret over the repaired session, or fails the launch.
pub(super) fn place<'machine>(
    session: RepairedHostControl<HostIo<'machine>>,
    secrets: &[SecretFile],
) -> Result<RepairedHostControl<HostIo<'machine>>, SessionError> {
    let (session, placement) = session
        .place_secrets(secrets)
        .map_err(|_| SessionError::Secret)?;
    match placement {
        SecretPlacement::Placed => Ok(session),
        // The guest answered, so the session is intact, but the sandbox does not hold what it
        // was launched with. Continuing would publish a Ready machine whose workload is going to
        // fail for a reason no receipt names.
        SecretPlacement::Refused { .. } => Err(SessionError::Secret),
    }
}
