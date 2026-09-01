//! The authenticated evidence one restored Instance requires before it is ready.
//! A restore resumes a machine whose guest has not yet repaired its identity, entropy, time, or
//! network state, so readiness is a claim the restore must be shown rather than one it may
//! assume. The evidence is a [`ReadinessReceipt`]: a keyed tag over the exact restored snapshot,
//! the exact launch authority this restore published, and the Instance, Launch operation, and
//! live handshake transcript of one authenticated guest session.
//! The transcript identifies that session; it is fixed at handshake completion and is unchanged
//! by repair or by the readiness probe, so the receipt attests session identity rather than
//! repair completion. A receipt minted before repair finished is byte-identical to one minted
//! after, and no verifier can tell them apart.
//! The Instance and Launch operation are read out of the published page rather than accepted
//! from the caller, so a receipt naming any other session is refused before its tag is even
//! compared.
//! The key is a fresh [`ReadinessChallenge`] the restore samples for itself. It is in no
//! snapshot, in no Generation, and on no launch page, it has no public constructor, and the
//! transition takes it before it verifies anything, so one restore accepts at most one attempt
//! and a receipt minted for one Instance can never complete another.

use core::fmt;

use super::{Digest, Hasher};

// The challenge is the half of this module that holds a secret. It lives beside this file so the
// non-secret evidence below can be read without reading it, and so a change to the tag cannot be
// made without opening the file whose whole subject is the tag.
pub use challenge::{ReadinessChallenge, ReadinessDemand};

mod challenge;

/// Why a readiness receipt did not complete one restore.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadinessRefusal {
    /// No fresh secret was available, so this restore can never be proved ready.
    Unavailable,
    /// A session identity or transcript was entirely zero, so it binds nothing.
    Unbound,
    /// No launch authority has been published yet, so there is nothing to bind a receipt to.
    Unpublished,
    /// This restore's single-use challenge was already spent by an earlier attempt.
    Spent,
    /// The receipt does not authenticate against this restore's challenge and identity.
    Rejected,
    /// The receipt names a session the published launch page does not bind.
    Foreign,
}

impl fmt::Display for ReadinessRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "readiness refused: {self:?}")
    }
}

/// Where the launch page carries the Instance and Launch operation it binds.
/// The page begins with its domain, schema version, authentication profile, and Generation
/// digest; the two session identities follow. `page_session` is proved against a real page
/// built by the guest crate, so a schema change cannot silently move the fields.
const INSTANCE_OFFSET: usize = 16 + 2 + 2 + 32;
const OPERATION_OFFSET: usize = INSTANCE_OFFSET + 16;

/// The Instance and Launch operation one launch page binds, in that order.
pub type PageSession = ([u8; 16], [u8; 16]);

/// Reads the Instance and Launch operation out of one published launch page.
#[must_use]
pub fn page_session(page: &[u8]) -> Option<PageSession> {
    let instance = page.get(INSTANCE_OFFSET..OPERATION_OFFSET)?;
    let operation = page.get(OPERATION_OFFSET..OPERATION_OFFSET + 16)?;
    let mut identities = ([0_u8; 16], [0_u8; 16]);
    identities.0.copy_from_slice(instance);
    identities.1.copy_from_slice(operation);
    Some(identities)
}

/// What one restore is: the snapshot it came from and the launch authority it published.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestoredIdentity {
    snapshot: Digest,
    launch: Digest,
    session: PageSession,
}

impl RestoredIdentity {
    /// Names one restored machine by its snapshot state object, its published launch page, and
    /// the Instance and Launch operation that page binds.
    /// The session identities are read out of the page itself rather than asserted by the
    /// caller, so a receipt that names any other session is refused.
    #[must_use]
    pub const fn new(snapshot: Digest, launch: Digest, session: PageSession) -> Self {
        Self {
            snapshot,
            launch,
            session,
        }
    }

    /// The digest of the exact snapshot state object this Instance was restored from.
    #[must_use]
    pub const fn snapshot(&self) -> &Digest {
        &self.snapshot
    }

    /// The digest of the exact launch page this restore published into the machine.
    #[must_use]
    pub const fn launch(&self) -> &Digest {
        &self.launch
    }
}

/// What one authenticated, repaired guest session is.
///
/// Every field is non-secret and already crosses the wire elsewhere; the session's authority
/// comes from the fact that only a repaired authenticated owner can produce this transcript.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionEvidence {
    instance: [u8; 16],
    operation: [u8; 16],
    transcript: [u8; 32],
}

impl SessionEvidence {
    /// Binds one receipt to the Instance, Launch operation, and transcript of one session.
    /// # Errors
    /// Returns [`ReadinessRefusal::Unbound`] when any field is entirely zero.
    pub fn new(
        instance: [u8; 16],
        operation: [u8; 16],
        transcript: [u8; 32],
    ) -> Result<Self, ReadinessRefusal> {
        if instance.iter().all(|byte| *byte == 0)
            || operation.iter().all(|byte| *byte == 0)
            || transcript.iter().all(|byte| *byte == 0)
        {
            return Err(ReadinessRefusal::Unbound);
        }
        Ok(Self {
            instance,
            operation,
            transcript,
        })
    }

    /// The fresh Instance this session authenticated.
    #[must_use]
    pub const fn instance(&self) -> &[u8; 16] {
        &self.instance
    }

    /// The Launch operation this session is bound to.
    #[must_use]
    pub const fn operation(&self) -> &[u8; 16] {
        &self.operation
    }
}

/// The evidence one restore consumes to become ready.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ReadinessReceipt {
    session: SessionEvidence,
    tag: [u8; 32],
}

impl ReadinessReceipt {
    /// The session this receipt was minted from.
    #[must_use]
    pub const fn session(&self) -> &SessionEvidence {
        &self.session
    }

    /// Returns this receipt with its tag replaced, for negative proofs only.
    #[must_use]
    #[cfg(test)]
    pub(crate) const fn with_tag(mut self, tag: [u8; 32]) -> Self {
        self.tag = tag;
        self
    }
}

impl fmt::Debug for ReadinessReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ReadinessReceipt([REDACTED])")
    }
}

#[cfg(test)]
mod tests;
