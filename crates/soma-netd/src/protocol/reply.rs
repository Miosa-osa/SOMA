//! Daemon reply frames.

use soma_guest::ActivationChallenge;

use super::array;
use crate::{BundleId, CleanupGeneration, Error};

/// One daemon reply.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Reply {
    /// The bundle is assigned; the launch values follow in the same order as the launch page.
    Claimed {
        /// The bundle.
        bundle: BundleId,
        /// Its generation.
        generation: CleanupGeneration,
        /// The exact `LaunchNetwork` fields: CID, generation, MAC, address, prefix,
        /// gateway, resolver, time sample.
        launch: [u8; 35],
        /// The single-use activation challenge bound to this assignment.
        activation: ActivationChallenge,
    },
    /// Activation completed.
    Activated,
    /// Release completed; `complete` reports the final verification.
    Released {
        /// Whether the final live inspection found nothing owned.
        complete: bool,
    },
    /// Reconciliation counts.
    Reconciled {
        /// Ledger entries whose kernel state matches.
        consistent: u32,
        /// Entries with missing kernel objects.
        drifted: u32,
        /// Released entries with lingering kernel objects.
        orphaned: u32,
        /// Kernel objects with no ledger owner.
        unowned: u32,
    },
    /// A typed failure code from [`super::error_code`].
    Failed(u16),
}

impl Reply {
    /// Encodes the reply.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        match self {
            Self::Claimed {
                bundle,
                generation,
                launch,
                activation,
            } => {
                out.push(1);
                out.extend_from_slice(bundle.as_bytes());
                out.extend_from_slice(&generation.get().to_be_bytes());
                out.extend_from_slice(launch);
                out.extend_from_slice(&activation.to_bytes());
            }
            Self::Activated => out.push(2),
            Self::Released { complete } => out.extend_from_slice(&[3, u8::from(*complete)]),
            Self::Reconciled {
                consistent,
                drifted,
                orphaned,
                unowned,
            } => {
                out.push(4);
                for value in [consistent, drifted, orphaned, unowned] {
                    out.extend_from_slice(&value.to_be_bytes());
                }
            }
            Self::Failed(code) => {
                out.push(0xff);
                out.extend_from_slice(&code.to_be_bytes());
            }
        }
        out
    }

    /// Decodes one exact reply frame.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Protocol`] for any malformed frame.
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        match (bytes.first(), bytes.len()) {
            (Some(1), 88) => Ok(Self::Claimed {
                bundle: BundleId::new(array(&bytes[1..17]))
                    .map_err(|_| Error::Protocol("bundle"))?,
                generation: CleanupGeneration::new(u32::from_be_bytes(array(&bytes[17..21])))
                    .map_err(|_| Error::Protocol("generation"))?,
                launch: array(&bytes[21..56]),
                activation: ActivationChallenge::from_bytes(array(&bytes[56..88]))
                    .map_err(|_| Error::Protocol("activation"))?,
            }),
            (Some(2), 1) => Ok(Self::Activated),
            (Some(3), 2) if bytes[1] <= 1 => Ok(Self::Released {
                complete: bytes[1] == 1,
            }),
            (Some(4), 17) => Ok(Self::Reconciled {
                consistent: u32::from_be_bytes(array(&bytes[1..5])),
                drifted: u32::from_be_bytes(array(&bytes[5..9])),
                orphaned: u32::from_be_bytes(array(&bytes[9..13])),
                unowned: u32::from_be_bytes(array(&bytes[13..17])),
            }),
            (Some(0xff), 3) => Ok(Self::Failed(u16::from_be_bytes([bytes[1], bytes[2]]))),
            _ => Err(Error::Protocol("reply")),
        }
    }
}
