//! Taking the link down at the suspension point (ADR 0034).
//!
//! The link is down across the gap so the transmit watchdog has no in-flight queue to time out
//! on while the machine is not running; the resume's repair raises it again with a fresh identity,
//! as it does at every launch.

use super::encoding::IFF_UP;
use super::{INTERFACE, NetworkError, NetworkStep, control_socket, get_flags, set_flags, target};

/// Takes `eth0` down. A machine with no network device has nothing to take down.
///
/// # Errors
///
/// Returns the failed step with its errno.
pub fn link_down(declared_net: bool) -> Result<(), NetworkError> {
    if !declared_net {
        return Ok(());
    }
    target::require(target::COMPILED)?;
    let socket = control_socket()?;
    let flags = get_flags(&socket, INTERFACE, NetworkStep::LinkDown)?;
    set_flags(&socket, INTERFACE, flags & !IFF_UP, NetworkStep::LinkDown)
}
