//! Daemon reply frames.

use super::{ProtocolError, array};
use crate::{LeaseGeneration, WorkerId};

/// One daemon reply.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Reply {
    /// Fresh authority was transferred; the launch-page network values follow in order.
    Claimed {
        /// The worker.
        worker: WorkerId,
        /// The lease generation.
        lease_generation: LeaseGeneration,
        /// CID, generation, MAC, address, prefix, gateway, resolver, time sample.
        launch: [u8; 35],
    },
    /// The operation already holds this worker, but this process did not deliver its launch
    /// page and cannot repeat it; the client releases the worker and launches again under a
    /// fresh operation.
    Replayed {
        /// The worker.
        worker: WorkerId,
        /// The lease generation.
        lease_generation: LeaseGeneration,
    },
    /// Release completed.
    Released {
        /// Whether teardown and release both reported completion.
        complete: bool,
    },
    /// One worker's phase and generation.
    Inspected {
        /// The phase code.
        phase: u8,
        /// The lease generation.
        lease_generation: LeaseGeneration,
    },
    /// Reconciliation counts.
    Reconciled {
        /// Suspects.
        suspects: u32,
        /// Terminated.
        terminated: u32,
        /// Released.
        released: u32,
        /// Retained.
        retained: u32,
    },
    /// A typed failure code from [`failure_code`].
    Failed(u16),
}

impl Reply {
    /// Encodes the reply.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        match self {
            Self::Claimed {
                worker,
                lease_generation,
                launch,
            } => {
                out.push(1);
                out.extend_from_slice(worker.as_bytes());
                out.extend_from_slice(&lease_generation.get().to_be_bytes());
                out.extend_from_slice(launch);
            }
            Self::Replayed {
                worker,
                lease_generation,
            } => {
                out.push(2);
                out.extend_from_slice(worker.as_bytes());
                out.extend_from_slice(&lease_generation.get().to_be_bytes());
            }
            Self::Released { complete } => out.extend_from_slice(&[3, u8::from(*complete)]),
            Self::Inspected {
                phase,
                lease_generation,
            } => {
                out.extend_from_slice(&[4, *phase]);
                out.extend_from_slice(&lease_generation.get().to_be_bytes());
            }
            Self::Reconciled {
                suspects,
                terminated,
                released,
                retained,
            } => {
                out.push(5);
                for value in [suspects, terminated, released, retained] {
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
    /// Returns [`ProtocolError`] for any malformed frame.
    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let worker =
            |slice: &[u8]| WorkerId::new(array(slice)).map_err(|_| ProtocolError("worker"));
        let generation = |slice: &[u8]| {
            LeaseGeneration::new(u64::from_be_bytes(array(slice)))
                .map_err(|_| ProtocolError("generation"))
        };
        match (bytes.first(), bytes.len()) {
            (Some(1), 60) => Ok(Self::Claimed {
                worker: worker(&bytes[1..17])?,
                lease_generation: generation(&bytes[17..25])?,
                launch: array(&bytes[25..60]),
            }),
            (Some(2), 25) => Ok(Self::Replayed {
                worker: worker(&bytes[1..17])?,
                lease_generation: generation(&bytes[17..25])?,
            }),
            (Some(3), 2) if bytes[1] <= 1 => Ok(Self::Released {
                complete: bytes[1] == 1,
            }),
            (Some(4), 10) => Ok(Self::Inspected {
                phase: bytes[1],
                lease_generation: generation(&bytes[2..10])?,
            }),
            (Some(5), 17) => Ok(Self::Reconciled {
                suspects: u32::from_be_bytes(array(&bytes[1..5])),
                terminated: u32::from_be_bytes(array(&bytes[5..9])),
                released: u32::from_be_bytes(array(&bytes[9..13])),
                retained: u32::from_be_bytes(array(&bytes[13..17])),
            }),
            (Some(0xff), 3) => Ok(Self::Failed(u16::from_be_bytes([bytes[1], bytes[2]]))),
            _ => Err(ProtocolError("reply")),
        }
    }
}
