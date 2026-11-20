//! The bounded typed daemon protocol carried in `SOCK_SEQPACKET` frames.
//!
//! One request frame produces one reply frame; a successful claim is followed by one
//! descriptor transfer frame from [`crate::TransferHeader`].
//! A claim reply carries the assignment's single-use activation challenge, and the matching
//! activate request must carry the receipt the repaired guest session minted from it.
//! No frame carries a path, shell text, or raw ruleset.

mod code;
mod reply;
mod version;

pub use code::error_code;
pub use reply::Reply;
pub use version::PROTOCOL_VERSION;

use soma_guest::ActivationReceipt;

use crate::{
    BundleId, CleanupGeneration, EgressClass, Error, InstanceId, MAX_ENCODED_INTENT, NetworkIntent,
    OperationId,
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
    /// Activate one assigned bundle with the repaired guest session's activation receipt.
    Activate {
        /// The bundle.
        bundle: BundleId,
        /// Its generation.
        generation: CleanupGeneration,
        /// The single-use capability minted from this assignment's activation challenge.
        receipt: ActivationReceipt,
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
    /// Open one TCP connection from inside an activated bundle's namespace to its own guest,
    /// and transfer the connected socket.
    Connect {
        /// The Instance the bundle was claimed for; it must match the ledger.
        instance: InstanceId,
        /// The bundle.
        bundle: BundleId,
        /// Its generation.
        generation: CleanupGeneration,
        /// The guest port; it must be one the broker was configured to allow.
        port: u16,
    },
    /// Change the egress of one activated bundle without restarting its guest.
    ///
    /// Only `Denied` and `PublicInternet` are admitted. `dns` allows the profile's resolvers and
    /// is refused with `Denied`.
    SetEgress {
        /// The Instance the bundle was claimed for; it must match the ledger.
        instance: InstanceId,
        /// The bundle.
        bundle: BundleId,
        /// Its generation.
        generation: CleanupGeneration,
        /// The egress class to put in force.
        egress: EgressClass,
        /// Whether DNS to the profile's resolvers is permitted.
        dns: bool,
    },
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
            Self::Activate {
                bundle,
                generation,
                receipt,
            } => {
                out.push(2);
                out.extend_from_slice(bundle.as_bytes());
                out.extend_from_slice(&generation.get().to_be_bytes());
                out.extend_from_slice(&receipt.to_bytes());
            }
            Self::Release { bundle, generation } => {
                out.push(3);
                out.extend_from_slice(bundle.as_bytes());
                out.extend_from_slice(&generation.get().to_be_bytes());
            }
            Self::Reconcile => out.push(4),
            Self::Connect {
                instance,
                bundle,
                generation,
                port,
            } => {
                out.push(5);
                out.extend_from_slice(instance.as_bytes());
                out.extend_from_slice(bundle.as_bytes());
                out.extend_from_slice(&generation.get().to_be_bytes());
                out.extend_from_slice(&port.to_be_bytes());
            }
            Self::SetEgress {
                instance,
                bundle,
                generation,
                egress,
                dns,
            } => {
                out.push(6);
                out.extend_from_slice(instance.as_bytes());
                out.extend_from_slice(bundle.as_bytes());
                out.extend_from_slice(&generation.get().to_be_bytes());
                out.push(egress.code());
                out.push(u8::from(*dns));
            }
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
            2 if bytes.len() == 21 + ActivationReceipt::LEN => Ok(Self::Activate {
                bundle: BundleId::new(array(&bytes[1..17]))
                    .map_err(|_| Error::Protocol("bundle"))?,
                generation: CleanupGeneration::new(u32::from_be_bytes(array(&bytes[17..21])))
                    .map_err(|_| Error::Protocol("generation"))?,
                receipt: ActivationReceipt::from_bytes(&array(&bytes[21..]))
                    .map_err(|_| Error::Protocol("receipt"))?,
            }),
            3 if bytes.len() == 21 => Ok(Self::Release {
                bundle: BundleId::new(array(&bytes[1..17]))
                    .map_err(|_| Error::Protocol("bundle"))?,
                generation: CleanupGeneration::new(u32::from_be_bytes(array(&bytes[17..21])))
                    .map_err(|_| Error::Protocol("generation"))?,
            }),
            4 if bytes.len() == 1 => Ok(Self::Reconcile),
            5 if bytes.len() == 39 => Ok(Self::Connect {
                instance: InstanceId::new(array(&bytes[1..17]))
                    .map_err(|_| Error::Protocol("instance"))?,
                bundle: BundleId::new(array(&bytes[17..33]))
                    .map_err(|_| Error::Protocol("bundle"))?,
                generation: CleanupGeneration::new(u32::from_be_bytes(array(&bytes[33..37])))
                    .map_err(|_| Error::Protocol("generation"))?,
                port: match u16::from_be_bytes(array(&bytes[37..39])) {
                    0 => return Err(Error::Protocol("port")),
                    port => port,
                },
            }),
            6 if bytes.len() == 39 => Ok(Self::SetEgress {
                instance: InstanceId::new(array(&bytes[1..17]))
                    .map_err(|_| Error::Protocol("instance"))?,
                bundle: BundleId::new(array(&bytes[17..33]))
                    .map_err(|_| Error::Protocol("bundle"))?,
                generation: CleanupGeneration::new(u32::from_be_bytes(array(&bytes[33..37])))
                    .map_err(|_| Error::Protocol("generation"))?,
                // Unrestricted is an operator-admitted class at claim time; it is never reachable
                // by a runtime change, so the frame cannot even name it.
                egress: match EgressClass::from_code(bytes[37]) {
                    Some(class @ (EgressClass::Denied | EgressClass::PublicInternet)) => class,
                    _ => return Err(Error::Protocol("egress")),
                },
                dns: match bytes[38] {
                    0 => false,
                    1 => true,
                    _ => return Err(Error::Protocol("dns")),
                },
            }),
            _ => Err(Error::Protocol("request")),
        }
    }
}

pub(super) fn array<const N: usize>(slice: &[u8]) -> [u8; N] {
    let mut out = [0; N];
    out.copy_from_slice(slice);
    out
}

#[cfg(test)]
mod tests;
