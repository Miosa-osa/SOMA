//! One secret value the host holds on behalf of one Instance.
//!
//! The value exists on the host only for as long as one launch needs it. It is never part of a
//! Template, a Template Lock, a Generation, a snapshot, a log, or a receipt, so the only way it
//! reaches a guest is the authenticated session, and the only way it leaves this process is the
//! one narrow accessor below.
//!
//! Everything else about the type exists to make an accidental disclosure impossible rather than
//! unlikely. There is no derived `Debug`, no `Display`, no `Clone`, and no comparison: a value
//! that cannot be formatted cannot be logged by mistake, and a value that cannot be compared
//! cannot leak its bytes through the time a comparison takes. The owned copy is zeroized when it
//! is dropped, so the host does not keep the value in freed memory after the launch that used it.

use core::fmt;

use zeroize::Zeroizing;

use crate::Error;

/// Largest secret value this crate will deliver.
///
/// A credential is a small thing, and the bound is what keeps one delivery one bounded unit of
/// work: a value this size fits the single filesystem write the session already carries, so no
/// caller has to reason about a secret arriving in pieces.
pub const MAX_SECRET_BYTES: usize = 4096;

/// The bytes of one secret, owned by the host and never rendered.
pub struct SecretValue(Zeroizing<Vec<u8>>);

impl SecretValue {
    /// Takes ownership of one secret value.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSecret`] for an empty value and for one above
    /// [`MAX_SECRET_BYTES`]. An empty value is refused because a program given an empty
    /// credential fails in a way that looks like a wrong credential rather than a missing one.
    pub fn new(bytes: Vec<u8>) -> Result<Self, Error> {
        if bytes.is_empty() || bytes.len() > MAX_SECRET_BYTES {
            return Err(Error::InvalidSecret);
        }
        Ok(Self(Zeroizing::new(bytes)))
    }

    /// How many bytes the value holds.
    ///
    /// The length is not the value, and a caller that has to bound a transfer needs it.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the value holds no bytes, which a constructed value never does.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The bytes themselves, for the one crate-internal step that puts them on the session.
    ///
    /// This is deliberately not public. Delivery is the only thing in this system that has any
    /// business reading a secret value, and it lives in this crate, so nothing outside can
    /// obtain the bytes at all.
    pub(crate) fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    /// Reports the length and never the bytes.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "SecretValue {{ {} bytes }}", self.0.len())
    }
}
