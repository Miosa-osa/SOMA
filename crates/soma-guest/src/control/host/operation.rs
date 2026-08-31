//! The identity of a request the caller does not name.
//!
//! An Execute identity names work the caller tracks across the whole system, so the caller mints
//! it. A filesystem or terminal identity exists only to pair one answer with one question, and a
//! caller forced to invent one could reuse a value this session has already spent and lose the
//! transport for it. A random identity is also not one the caller could have chosen for an
//! Execute or a Shutdown, so minting one here never spends a value the caller still needed.

use crate::OperationId;

/// Mints one request identity from operating-system randomness.
///
/// Returns `None` only when local randomness is unavailable, and a session that cannot name its
/// next request can no longer tell an answer from a replay of an older one.
pub(super) fn fresh_operation() -> Option<OperationId> {
    let mut bytes = [0_u8; 16];
    crate::resolver::fill_os_random(&mut bytes).ok()?;
    OperationId::new(bytes).ok()
}
