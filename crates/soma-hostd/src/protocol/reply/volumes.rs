//! The volume reply codec of ADR 0053 Step A, split out to hold the reply file inside its budget.
//!
//! Every volume reply is fixed width per tag, so a frame whose length does not match its tag is
//! refused rather than read as a shorter valid one.

use super::{ProtocolError, array};
use crate::VolumeId;
use crate::protocol::Reply;

/// Encodes one volume reply, whose body is a discriminant, a volume, and at most five more bytes.
pub(super) fn encode_volume(out: &mut Vec<u8>, reply: &Reply) {
    match reply {
        Reply::VolumeAttached {
            volume,
            state,
            repair,
            exit_code,
        } => {
            out.push(10);
            out.extend_from_slice(volume.as_bytes());
            out.extend_from_slice(&[*state, *repair, exit_code.cast_unsigned()]);
        }
        Reply::VolumeDetached { volume, state } => {
            out.push(11);
            out.extend_from_slice(volume.as_bytes());
            out.extend_from_slice(&[*state]);
        }
        Reply::VolumeDestroyed { volume, state } => {
            out.push(12);
            out.extend_from_slice(volume.as_bytes());
            out.extend_from_slice(&[*state]);
        }
        Reply::VolumeInspected {
            volume,
            state,
            size_class_gib,
        } => {
            out.push(13);
            out.extend_from_slice(volume.as_bytes());
            out.push(*state);
            out.extend_from_slice(&size_class_gib.to_be_bytes());
        }
        // The caller matches the volume variants before it reaches here.
        _ => {}
    }
}

/// Decodes one volume reply, whose tag the caller has already matched.
pub(super) fn decode_volume(bytes: &[u8]) -> Result<Reply, ProtocolError> {
    let volume = |slice: &[u8]| VolumeId::new(array(slice)).map_err(|_| ProtocolError("volume"));
    match bytes.len() {
        20 if bytes[0] == 10 => Ok(Reply::VolumeAttached {
            volume: volume(&bytes[1..17])?,
            state: bytes[17],
            repair: bytes[18],
            exit_code: bytes[19].cast_signed(),
        }),
        18 if bytes[0] == 11 => Ok(Reply::VolumeDetached {
            volume: volume(&bytes[1..17])?,
            state: bytes[17],
        }),
        18 if bytes[0] == 12 => Ok(Reply::VolumeDestroyed {
            volume: volume(&bytes[1..17])?,
            state: bytes[17],
        }),
        22 if bytes[0] == 13 => Ok(Reply::VolumeInspected {
            volume: volume(&bytes[1..17])?,
            state: bytes[17],
            size_class_gib: u32::from_be_bytes(array(&bytes[18..22])),
        }),
        _ => Err(ProtocolError("reply")),
    }
}
