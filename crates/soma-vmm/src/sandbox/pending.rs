//! What a repaired guest session must mint before its network may carry traffic.
//!
//! The receipt can only be produced from inside the session, and the privileged broker will
//! only accept it from the peer that claimed the assignment. So the challenge travels into the
//! sandbox and the receipt travels back out, and neither side can raise the link alone.

use soma_guest::ActivationChallenge;

/// The broker's demand on one Instance's session, carried into the sandbox that will answer it.
pub struct PendingActivation {
    /// The broker's fresh secret for this assignment.
    pub challenge: ActivationChallenge,
    /// The assignment generation the receipt is bound to.
    pub generation: u32,
    /// The digest of the admitted intent the receipt is bound to.
    pub intent: [u8; 32],
}
