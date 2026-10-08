//! The fresh secret one restore samples for itself, and the keyed tag it authenticates a
//! readiness receipt with.
//!
//! This is the half of readiness that holds a secret. Everything in the parent module is
//! non-secret and already crosses the wire elsewhere; the challenge is in no snapshot, in no
//! Generation, and on no launch page, and it has no public constructor. Keeping it here means
//! the tag construction, which is what an attacker would have to forge, is one small file whose
//! whole subject is that construction.

use core::fmt;

#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
use super::ReadinessRefusal;
use super::{Hasher, ReadinessReceipt, RestoredIdentity, SessionEvidence};

const DOMAIN: &[u8; 24] = b"SOMA-RESTORE-READINESS\0\0";
const SCHEMA_VERSION: u16 = 1;
const BLOCK: usize = 64;

/// The fresh single-use secret one restore requires in its readiness receipt.
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
    #[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
    pub(crate) fn adopt(bytes: [u8; 32]) -> Result<Self, ReadinessRefusal> {
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(ReadinessRefusal::Unavailable);
        }
        Ok(Self(bytes))
    }

    /// Requires one receipt to authenticate against this challenge and exact identity.
    #[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
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

impl ReadinessDemand<'_> {
    #[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
    pub(crate) const fn new(
        challenge: &ReadinessChallenge,
        identity: RestoredIdentity,
    ) -> ReadinessDemand<'_> {
        ReadinessDemand {
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

#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
fn equal(left: &[u8; 32], right: &[u8; 32]) -> bool {
    let mut difference = 0_u8;
    for (own, other) in left.iter().zip(right.iter()) {
        difference |= own ^ other;
    }
    difference == 0
}
