//! The single-use network-activation capability one claiming peer presents.
//!
//! The privileged network broker samples one fresh [`ActivationChallenge`] while it assigns a
//! network bundle and returns it only to the peer that claimed that assignment.
//! [`crate::RepairedHostControl`], which exists only after authenticated repair succeeded,
//! converts that challenge into an [`ActivationReceipt`].
//!
//! The receipt is a keyed tag over the exact [`ActivationScope`] and the live Noise transcript,
//! so it cannot be moved to another Instance, assignment generation, Launch operation, or
//! admitted network intent, and the broker consumes its challenge exactly once.
//!
//! What this proves, exactly: the party presenting the receipt is the party the broker gave the
//! challenge to, and it presents it once. It is not guest evidence. The challenge is the only
//! secret in the scheme, the broker generates it and hands it to the claiming peer in cleartext,
//! and the transcript half is carried by the receipt rather than known to the broker, so any
//! holder of the challenge can compute an accepted receipt with no guest session at all.
//! Making this capability unforgeable by its presenter needs a secret the presenter does not
//! hold - a guest-held key the broker can verify against - which does not exist yet.

use core::fmt;

use zeroize::Zeroizing;

use crate::{Error, resolver};

const DOMAIN: &[u8; 24] = b"SOMA-NETWORK-ACTIVATION\0";
const SCHEMA_VERSION: u16 = 1;
const MESSAGE_LEN: usize = DOMAIN.len() + 2 + 16 + 16 + 4 + 32 + 32;

/// The exact assignment one activation capability may enable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivationScope {
    instance: [u8; 16],
    operation: [u8; 16],
    generation: u32,
    intent: [u8; 32],
}

impl ActivationScope {
    /// Binds one activation capability to an exact network assignment.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidActivationScope`] for a zero identity, generation, or digest.
    pub fn new(
        instance: [u8; 16],
        operation: [u8; 16],
        generation: u32,
        intent: [u8; 32],
    ) -> Result<Self, Error> {
        if instance.iter().all(|byte| *byte == 0)
            || operation.iter().all(|byte| *byte == 0)
            || generation == 0
            || intent.iter().all(|byte| *byte == 0)
        {
            return Err(Error::InvalidActivationScope);
        }
        Ok(Self {
            instance,
            operation,
            generation,
            intent,
        })
    }

    fn message(&self, transcript: &[u8; 32]) -> [u8; MESSAGE_LEN] {
        let mut encoded = [0_u8; MESSAGE_LEN];
        let mut cursor = 0;
        for field in [
            DOMAIN.as_slice(),
            &SCHEMA_VERSION.to_be_bytes(),
            &self.instance,
            &self.operation,
            &self.generation.to_be_bytes(),
            &self.intent,
            transcript,
        ] {
            let end = cursor + field.len();
            encoded[cursor..end].copy_from_slice(field);
            cursor = end;
        }
        debug_assert_eq!(cursor, MESSAGE_LEN);
        encoded
    }
}

/// The fresh single-use secret one broker assignment binds to its activation.
pub struct ActivationChallenge(Zeroizing<[u8; 32]>);

impl ActivationChallenge {
    /// Samples one fresh challenge from operating-system randomness.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RandomnessUnavailable`] when no fresh nonzero secret is available.
    pub fn generate() -> Result<Self, Error> {
        let mut bytes = Zeroizing::new([0_u8; 32]);
        resolver::fill_os_random(bytes.as_mut())?;
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(Error::RandomnessUnavailable);
        }
        Ok(Self(bytes))
    }

    /// Reconstructs the challenge one authenticated broker reply delivered.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidKeyMaterial`] for the all-zero value.
    pub fn from_bytes(bytes: [u8; 32]) -> Result<Self, Error> {
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(Error::InvalidKeyMaterial);
        }
        Ok(Self(Zeroizing::new(bytes)))
    }

    /// Returns the bearer bytes for delivery over the authenticated broker connection.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; 32] {
        *self.0
    }

    /// Requires one receipt to authenticate against this challenge and exact scope.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ActivationReceiptRejected`] when the tag does not match, and
    /// [`Error::CryptoSetup`] when the fixed suite is unavailable.
    pub fn verify(
        &self,
        scope: &ActivationScope,
        receipt: &ActivationReceipt,
    ) -> Result<(), Error> {
        let expected = self.tag(scope, &receipt.transcript)?;
        if equal(&expected, &receipt.tag) {
            Ok(())
        } else {
            Err(Error::ActivationReceiptRejected)
        }
    }

    pub(crate) fn tag(
        &self,
        scope: &ActivationScope,
        transcript: &[u8; 32],
    ) -> Result<[u8; 32], Error> {
        resolver::keyed_tag(&self.0, &scope.message(transcript))
    }
}

/// The single-use capability one repaired authenticated session minted.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ActivationReceipt {
    transcript: [u8; 32],
    tag: [u8; 32],
}

impl ActivationReceipt {
    /// The exact encoded receipt length.
    pub const LEN: usize = 64;

    pub(crate) const fn new(transcript: [u8; 32], tag: [u8; 32]) -> Self {
        Self { transcript, tag }
    }

    /// Encodes the receipt for one broker request.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; Self::LEN] {
        let mut encoded = [0_u8; Self::LEN];
        encoded[..32].copy_from_slice(&self.transcript);
        encoded[32..].copy_from_slice(&self.tag);
        encoded
    }

    /// Decodes one exact receipt.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ActivationReceiptRejected`] for an all-zero transcript or tag.
    pub fn from_bytes(bytes: &[u8; Self::LEN]) -> Result<Self, Error> {
        let mut transcript = [0_u8; 32];
        let mut tag = [0_u8; 32];
        transcript.copy_from_slice(&bytes[..32]);
        tag.copy_from_slice(&bytes[32..]);
        if transcript.iter().all(|byte| *byte == 0) || tag.iter().all(|byte| *byte == 0) {
            return Err(Error::ActivationReceiptRejected);
        }
        Ok(Self { transcript, tag })
    }

    /// Returns the authenticated session transcript this receipt was minted from.
    #[must_use]
    pub const fn transcript(&self) -> &[u8; 32] {
        &self.transcript
    }
}

fn equal(left: &[u8; 32], right: &[u8; 32]) -> bool {
    let mut difference = 0_u8;
    for (own, other) in left.iter().zip(right.iter()) {
        difference |= own ^ other;
    }
    difference == 0
}

impl PartialEq for ActivationChallenge {
    fn eq(&self, other: &Self) -> bool {
        equal(&self.0, &other.0)
    }
}

impl Eq for ActivationChallenge {}

impl Clone for ActivationChallenge {
    fn clone(&self) -> Self {
        Self(Zeroizing::new(*self.0))
    }
}

macro_rules! redacted_debug {
    ($type:ty, $name:literal) => {
        impl fmt::Debug for $type {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!($name, "([REDACTED])"))
            }
        }
    };
}

redacted_debug!(ActivationChallenge, "ActivationChallenge");
redacted_debug!(ActivationReceipt, "ActivationReceipt");

#[cfg(test)]
mod tests;
