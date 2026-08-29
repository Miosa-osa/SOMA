//! Transfer of exactly one TAP descriptor over `AF_UNIX SOCK_SEQPACKET` with `SCM_RIGHTS`.
//!
//! One datagram carries a fixed typed header and one descriptor.
//! The receiver rejects a wrong magic, version, length, zero identity, any descriptor count
//! other than one, and a truncated control message, closing every stray descriptor.

use crate::{BundleId, CleanupGeneration, Error, IntentDigest, TransferRejection};

#[cfg(target_os = "linux")]
mod scm;

#[cfg(target_os = "linux")]
pub use scm::{receive_tap, send_tap, seqpacket_pair};

const MAGIC: &[u8; 8] = b"SOMATAP\0";
const VERSION: u16 = 1;

/// The exact encoded header length.
pub const MAX_HEADER: usize = 8 + 2 + 16 + 4 + 32;

/// The typed header that accompanies one TAP descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransferHeader {
    /// The bundle the TAP belongs to.
    pub bundle: BundleId,
    /// The cleanup generation of the assignment.
    pub generation: CleanupGeneration,
    /// The digest of the admitted intent.
    pub intent: IntentDigest,
}

impl TransferHeader {
    /// Encodes the header.
    #[must_use]
    pub fn encode(&self) -> [u8; MAX_HEADER] {
        let mut out = [0; MAX_HEADER];
        out[..8].copy_from_slice(MAGIC);
        out[8..10].copy_from_slice(&VERSION.to_be_bytes());
        out[10..26].copy_from_slice(self.bundle.as_bytes());
        out[26..30].copy_from_slice(&self.generation.get().to_be_bytes());
        out[30..].copy_from_slice(&self.intent.0);
        out
    }

    /// Decodes one exact header.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Transfer`] naming the first rejected field.
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        use TransferRejection as R;
        if bytes.len() != MAX_HEADER {
            return Err(Error::Transfer(R::BadLength));
        }
        if &bytes[..8] != MAGIC {
            return Err(Error::Transfer(R::BadMagic));
        }
        if u16::from_be_bytes([bytes[8], bytes[9]]) != VERSION {
            return Err(Error::Transfer(R::BadVersion));
        }
        let mut bundle = [0; 16];
        bundle.copy_from_slice(&bytes[10..26]);
        let bundle = BundleId::new(bundle).map_err(|_| Error::Transfer(R::ZeroBundle))?;
        let generation = u32::from_be_bytes([bytes[26], bytes[27], bytes[28], bytes[29]]);
        let generation =
            CleanupGeneration::new(generation).map_err(|_| Error::Transfer(R::ZeroGeneration))?;
        let mut intent = [0; 32];
        intent.copy_from_slice(&bytes[30..]);
        Ok(Self {
            bundle,
            generation,
            intent: IntentDigest(intent),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn header() -> TransferHeader {
        TransferHeader {
            bundle: BundleId::new([5; 16]).expect("id"),
            generation: CleanupGeneration::new(3).expect("generation"),
            intent: IntentDigest([9; 32]),
        }
    }

    type Mutation = Box<dyn Fn(&mut [u8; MAX_HEADER])>;

    #[test]
    fn header_round_trips_and_rejects_every_hostile_class() {
        let encoded = header().encode();
        assert_eq!(TransferHeader::decode(&encoded).expect("decodes"), header());
        let cases: [(Mutation, TransferRejection); 4] = [
            (Box::new(|b| b[0] = b'X'), TransferRejection::BadMagic),
            (Box::new(|b| b[9] = 2), TransferRejection::BadVersion),
            (
                Box::new(|b| b[10..26].fill(0)),
                TransferRejection::ZeroBundle,
            ),
            (
                Box::new(|b| b[26..30].fill(0)),
                TransferRejection::ZeroGeneration,
            ),
        ];
        for (mutate, expected) in cases {
            let mut bytes = encoded;
            mutate(&mut bytes);
            assert_eq!(
                TransferHeader::decode(&bytes).expect_err("rejected"),
                Error::Transfer(expected)
            );
        }
        assert_eq!(
            TransferHeader::decode(&encoded[..MAX_HEADER - 1]).expect_err("short"),
            Error::Transfer(TransferRejection::BadLength)
        );
        let mut long = encoded.to_vec();
        long.push(0);
        assert_eq!(
            TransferHeader::decode(&long).expect_err("long"),
            Error::Transfer(TransferRejection::BadLength)
        );
    }
}
