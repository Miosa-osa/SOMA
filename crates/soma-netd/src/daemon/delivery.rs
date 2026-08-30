//! Complete reply delivery, or a terminal protocol failure.
//!
//! A `SOCK_SEQPACKET` reply is exactly one datagram, so delivery either places the complete
//! frame in the peer's queue or does not happen at all.
//! Anything else, including a refused send, a short send, or a peer whose queue is full, is a
//! terminal protocol failure for that connection rather than a result the broker can continue
//! from.
//! The send never blocks, so a peer that stops reading disconnects itself instead of wedging
//! the single-threaded broker.
//!
//! Every lifecycle mutation commits, and its durable ledger record is written, before its reply
//! is delivered, so an undelivered reply never means the mutation did not happen.
//! The peer recovers by operation identity: a lost `Claimed` reply is recovered by replaying
//! the same Instance and Launch operation, which returns that same assignment; a lost
//! `Activated` reply leaves a spent challenge that fails closed on replay; a lost `Released`
//! reply is recovered by replaying the idempotent release; and reconciliation compares the
//! ledger with the kernel after any restart.

use std::os::fd::{AsRawFd, BorrowedFd};

use crate::Error;

/// Sends one complete reply frame or names the terminal protocol failure.
///
/// # Errors
///
/// Returns [`Error::Protocol`] when the peer did not receive the complete frame.
pub(super) fn deliver(connection: BorrowedFd<'_>, bytes: &[u8]) -> Result<(), Error> {
    // SAFETY: `bytes` is a valid buffer for its full length; the flags make the call
    // non-blocking and keep a closed peer from raising `SIGPIPE`.
    let sent = unsafe {
        libc::send(
            connection.as_raw_fd(),
            bytes.as_ptr().cast(),
            bytes.len(),
            libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT,
        )
    };
    if usize::try_from(sent).is_ok_and(|sent| sent == bytes.len()) {
        Ok(())
    } else {
        Err(Error::Protocol("reply delivery"))
    }
}

#[cfg(test)]
mod tests;
