//! The bounded typed daemon protocol carried in `SOCK_SEQPACKET` frames.
//!
//! One request frame produces one reply frame; a successful claim is followed by one
//! descriptor transfer frame from [`crate::TransferHeader`].
//! No frame carries a path, shell text, or raw ruleset.

mod reply;

pub use reply::Reply;

use crate::{
    BundleId, CleanupGeneration, Error, InstanceId, MAX_ENCODED_INTENT, NetworkIntent, OperationId,
};

/// The largest request or reply frame.
pub const MAX_FRAME: usize = 1 + 16 + 16 + 4 + MAX_ENCODED_INTENT;

/// One client request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Request {
    /// Claim one prepared bundle for an Instance.
    Claim {
        /// The owning Instance.
        instance: InstanceId,
        /// The idempotent operation.
        operation: OperationId,
        /// The vsock CID the caller's allocator assigned.
        vsock_cid: u32,
        /// The admitted intent.
        intent: NetworkIntent,
    },
    /// Activate one assigned bundle after authenticated repair.
    Activate {
        /// The bundle.
        bundle: BundleId,
        /// Its generation.
        generation: CleanupGeneration,
    },
    /// Release one bundle.
    Release {
        /// The bundle.
        bundle: BundleId,
        /// Its generation.
        generation: CleanupGeneration,
    },
    /// Compare the ledger with the kernel.
    Reconcile,
}

impl Request {
    /// Encodes the request.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(MAX_FRAME);
        match self {
            Self::Claim {
                instance,
                operation,
                vsock_cid,
                intent,
            } => {
                out.push(1);
                out.extend_from_slice(instance.as_bytes());
                out.extend_from_slice(operation.as_bytes());
                out.extend_from_slice(&vsock_cid.to_be_bytes());
                out.extend_from_slice(&intent.encode());
            }
            Self::Activate { bundle, generation } => {
                out.push(2);
                out.extend_from_slice(bundle.as_bytes());
                out.extend_from_slice(&generation.get().to_be_bytes());
            }
            Self::Release { bundle, generation } => {
                out.push(3);
                out.extend_from_slice(bundle.as_bytes());
                out.extend_from_slice(&generation.get().to_be_bytes());
            }
            Self::Reconcile => out.push(4),
        }
        out
    }

    /// Decodes one exact request frame.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Protocol`] for any malformed frame.
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.is_empty() || bytes.len() > MAX_FRAME {
            return Err(Error::Protocol("frame length"));
        }
        match bytes[0] {
            1 if bytes.len() > 37 => Ok(Self::Claim {
                instance: InstanceId::new(array(&bytes[1..17]))
                    .map_err(|_| Error::Protocol("instance"))?,
                operation: OperationId::new(array(&bytes[17..33]))
                    .map_err(|_| Error::Protocol("operation"))?,
                vsock_cid: u32::from_be_bytes(array(&bytes[33..37])),
                intent: NetworkIntent::decode(&bytes[37..])?,
            }),
            2 | 3 if bytes.len() == 21 => {
                let bundle =
                    BundleId::new(array(&bytes[1..17])).map_err(|_| Error::Protocol("bundle"))?;
                let generation = CleanupGeneration::new(u32::from_be_bytes(array(&bytes[17..21])))
                    .map_err(|_| Error::Protocol("generation"))?;
                Ok(if bytes[0] == 2 {
                    Self::Activate { bundle, generation }
                } else {
                    Self::Release { bundle, generation }
                })
            }
            4 if bytes.len() == 1 => Ok(Self::Reconcile),
            _ => Err(Error::Protocol("request")),
        }
    }
}

/// Maps one broker failure onto a stable protocol code.
#[must_use]
pub fn error_code(error: &Error) -> u16 {
    match error {
        Error::InvalidIntent(_) => 1,
        Error::InvalidProfile(_) => 2,
        Error::PoolExhausted => 3,
        Error::InvalidId(_) => 4,
        Error::MissingPrivilege(_) => 5,
        Error::Kernel { .. } => 6,
        Error::Tool { .. } => 7,
        Error::LedgerConflict => 8,
        Error::ReplayMismatch => 9,
        Error::LedgerCorrupt => 10,
        Error::NotAssigned => 11,
        Error::Drift(_) => 12,
        Error::Transfer(_) => 13,
        Error::PortUnavailable => 14,
        Error::Unimplemented(_) => 15,
        Error::InvalidState(_) => 16,
        Error::Protocol(_) => 17,
    }
}

pub(super) fn array<const N: usize>(slice: &[u8]) -> [u8; N] {
    let mut out = [0; N];
    out.copy_from_slice(slice);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EgressClass, ProfileDigest};

    #[test]
    fn requests_and_replies_round_trip_and_reject_hostile_frames() {
        let intent = NetworkIntent::new(
            EgressClass::Denied,
            Vec::new(),
            Vec::new(),
            ProfileDigest([1; 32]),
        )
        .expect("intent");
        let bundle = BundleId::new([3; 16]).expect("id");
        let generation = CleanupGeneration::new(2).expect("g");
        let requests = [
            Request::Claim {
                instance: InstanceId::new([1; 16]).expect("id"),
                operation: OperationId::new([2; 16]).expect("id"),
                vsock_cid: 7,
                intent,
            },
            Request::Activate { bundle, generation },
            Request::Release { bundle, generation },
            Request::Reconcile,
        ];
        for request in requests {
            let encoded = request.encode();
            assert!(encoded.len() <= MAX_FRAME);
            assert_eq!(Request::decode(&encoded).expect("decodes"), request);
            let mut extended = encoded.clone();
            extended.push(0);
            assert!(Request::decode(&extended).is_err());
        }
        let replies = [
            Reply::Claimed {
                bundle,
                generation,
                launch: [9; 35],
            },
            Reply::Activated,
            Reply::Released { complete: true },
            Reply::Reconciled {
                consistent: 1,
                drifted: 2,
                orphaned: 3,
                unowned: 4,
            },
            Reply::Failed(error_code(&Error::PoolExhausted)),
        ];
        for reply in replies {
            assert_eq!(Reply::decode(&reply.encode()).expect("decodes"), reply);
        }
        assert_eq!(Request::decode(&[]), Err(Error::Protocol("frame length")));
        assert_eq!(Request::decode(&[9]), Err(Error::Protocol("request")));
        assert_eq!(Reply::decode(&[3, 2]), Err(Error::Protocol("reply")));
        assert_eq!(
            Request::decode(&[1; 38]).expect_err("bad intent"),
            Error::Protocol("short")
        );
    }
}
