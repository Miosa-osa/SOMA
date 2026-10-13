//! Encoding and decoding the reply frames.
//!
//! It lives apart from the reply enum so that file stays inside the source budget; the wire is
//! the same wire and every arm is the arm that was there.

use super::volumes::{decode_volume, encode_volume};
use super::{ProtocolError, Reply, array, listed};
use crate::{LeaseGeneration, MAX_LISTED, WorkerId};

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
            Self::Launched {
                worker,
                lease_generation,
                launch,
            } => {
                out.push(6);
                out.extend_from_slice(worker.as_bytes());
                out.extend_from_slice(&lease_generation.get().to_be_bytes());
                out.extend_from_slice(launch);
            }
            Self::Live {
                worker,
                lease_generation,
                phase,
            } => {
                out.extend_from_slice(&[7, *phase]);
                out.extend_from_slice(worker.as_bytes());
                out.extend_from_slice(&lease_generation.get().to_be_bytes());
            }
            Self::Listed { instances, more } => {
                // The count is written before the identities so a short frame cannot be read
                // as a complete page of fewer Instances.
                let listed = &instances[..instances.len().min(MAX_LISTED)];
                let count = u8::try_from(listed.len()).unwrap_or(0);
                out.extend_from_slice(&[8, u8::from(*more), count]);
                for instance in listed {
                    out.extend_from_slice(instance.as_bytes());
                }
            }
            Self::Destroyed { worker, complete } => {
                out.extend_from_slice(&[9, u8::from(*complete)]);
                out.extend_from_slice(worker.as_bytes());
            }
            Self::VolumeAttached { .. }
            | Self::VolumeDetached { .. }
            | Self::VolumeDestroyed { .. }
            | Self::VolumeInspected { .. }
            | Self::VolumeOpened { .. }
            | Self::VolumeOwned { .. } => encode_volume(&mut out, self),
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
            (Some(6), 60) => Ok(Self::Launched {
                worker: worker(&bytes[1..17])?,
                lease_generation: generation(&bytes[17..25])?,
                launch: array(&bytes[25..60]),
            }),
            (Some(7), 26) => Ok(Self::Live {
                phase: bytes[1],
                worker: worker(&bytes[2..18])?,
                lease_generation: generation(&bytes[18..26])?,
            }),
            (Some(8), _) => listed(bytes),
            (Some(9), 18) if bytes[1] <= 1 => Ok(Self::Destroyed {
                complete: bytes[1] == 1,
                worker: worker(&bytes[2..18])?,
            }),
            (Some(10..=15), _) => decode_volume(bytes),
            (Some(0xff), 3) => Ok(Self::Failed(u16::from_be_bytes([bytes[1], bytes[2]]))),
            _ => Err(ProtocolError("reply")),
        }
    }
}
