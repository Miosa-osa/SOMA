//! The authenticated evidence one restored Instance requires before it is ready.
//!
//! A restore resumes a machine whose guest has not yet repaired its identity, entropy, time, or
//! network state, so readiness is a claim the restore must be shown rather than one it may
//! assume. The evidence is a [`ReadinessReceipt`]: a keyed tag over the exact restored snapshot,
//! the exact launch authority this restore published, and the Instance, Launch operation, and
//! live handshake transcript of one authenticated guest session that completed repair and the
//! fixed readiness probe.
//! The Instance and Launch operation are read out of the published page rather than accepted
//! from the caller, so a receipt naming any other session is refused before its tag is even
//! compared.
//!
//! The key is a fresh [`ReadinessChallenge`] the restore samples for itself. It is in no
//! snapshot, in no Generation, and on no launch page, it has no public constructor, and the
//! transition takes it before it verifies anything, so one restore accepts at most one attempt
//! and a receipt minted for one Instance can never complete another.

use core::fmt;

use super::{Digest, Hasher};

const DOMAIN: &[u8; 24] = b"SOMA-RESTORE-READINESS\0\0";
const SCHEMA_VERSION: u16 = 1;
const BLOCK: usize = 64;

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
///
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
    ///
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
    ///
    /// # Errors
    ///
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

/// The fresh single-use secret one restore requires in its readiness receipt.
///
/// The type has no public constructor, so the only readiness receipt that can complete a
/// restore is one minted from that restore's own [`ReadinessDemand`].
pub struct ReadinessChallenge([u8; 32]);

impl ReadinessChallenge {
    /// Adopts one fresh sample as this restore's challenge.
    ///
    /// # Errors
    ///
    /// Returns [`ReadinessRefusal::Unavailable`] for the all-zero sample, which is what an
    /// entropy source that produced nothing looks like.
    pub(crate) fn adopt(bytes: [u8; 32]) -> Result<Self, ReadinessRefusal> {
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(ReadinessRefusal::Unavailable);
        }
        Ok(Self(bytes))
    }

    /// Requires one receipt to authenticate against this challenge and exact identity.
    pub(crate) fn accepts(
        &self,
        identity: &RestoredIdentity,
        receipt: &ReadinessReceipt,
    ) -> Result<(), ReadinessRefusal> {
        if identity.session != (receipt.session.instance, receipt.session.operation) {
            return Err(ReadinessRefusal::Foreign);
        }
        if equal(&self.tag(identity, &receipt.session), &receipt.tag) {
            Ok(())
        } else {
            Err(ReadinessRefusal::Rejected)
        }
    }

    fn tag(&self, identity: &RestoredIdentity, session: &SessionEvidence) -> [u8; 32] {
        let mut message = Vec::with_capacity(DOMAIN.len() + 2 + 32 + 32 + 16 + 16 + 32);
        message.extend_from_slice(DOMAIN);
        message.extend_from_slice(&SCHEMA_VERSION.to_be_bytes());
        message.extend_from_slice(identity.snapshot.as_bytes());
        message.extend_from_slice(identity.launch.as_bytes());
        message.extend_from_slice(&session.instance);
        message.extend_from_slice(&session.operation);
        message.extend_from_slice(&session.transcript);
        keyed_tag(&self.0, &message)
    }
}

/// Everything one guest-session owner needs to mint the receipt a restore demands.
#[derive(Debug)]
pub struct ReadinessDemand<'a> {
    challenge: &'a ReadinessChallenge,
    identity: RestoredIdentity,
}

impl<'a> ReadinessDemand<'a> {
    pub(crate) const fn new(challenge: &'a ReadinessChallenge, identity: RestoredIdentity) -> Self {
        Self {
            challenge,
            identity,
        }
    }

    /// The restored snapshot and launch authority every receipt for this restore must bind.
    #[must_use]
    pub const fn identity(&self) -> &RestoredIdentity {
        &self.identity
    }

    /// Mints the receipt this restore requires from one authenticated repaired session.
    #[must_use]
    pub fn attest(&self, session: &SessionEvidence) -> ReadinessReceipt {
        ReadinessReceipt {
            session: *session,
            tag: self.challenge.tag(&self.identity, session),
        }
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

impl fmt::Debug for ReadinessChallenge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ReadinessChallenge([REDACTED])")
    }
}

/// HMAC-SHA-256 over the snapshot module's own SHA-256, so no second hash suite appears.
fn keyed_tag(key: &[u8; 32], message: &[u8]) -> [u8; 32] {
    let mut inner_pad = [0x36_u8; BLOCK];
    let mut outer_pad = [0x5c_u8; BLOCK];
    for (index, byte) in key.iter().enumerate() {
        inner_pad[index] ^= *byte;
        outer_pad[index] ^= *byte;
    }
    let mut inner = Hasher::new();
    inner.update(&inner_pad);
    inner.update(message);
    let inner = inner.finish();
    let mut outer = Hasher::new();
    outer.update(&outer_pad);
    outer.update(inner.as_bytes());
    *outer.finish().as_bytes()
}

fn equal(left: &[u8; 32], right: &[u8; 32]) -> bool {
    let mut difference = 0_u8;
    for (own, other) in left.iter().zip(right.iter()) {
        difference |= own ^ other;
    }
    difference == 0
}

#[cfg(test)]
mod tests;
