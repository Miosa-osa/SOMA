//! The volume reply codec of ADR 0053 Step A, split out to hold the reply file inside its budget.
//!
//! Every volume reply is fixed width per tag, so a frame whose length does not match its tag is
//! refused rather than read as a shorter valid one.

use super::{ProtocolError, array};
use crate::VolumeId;
use crate::protocol::Reply;

/// The widest volume path one reply carries.
pub const MAX_VOLUME_PATH_BYTES: usize = 256;
/// The fixed length of one opened-volume reply: tag, volume, class, repair, and the path field.
const OPENED_FRAME_LEN: usize = 1 + 16 + 4 + 1 + 1 + 2 + MAX_VOLUME_PATH_BYTES;

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
        Reply::VolumeOpened {
            volume,
            size_class_gib,
            repair,
            exit_code,
            path,
        } => {
            out.push(14);
            out.extend_from_slice(volume.as_bytes());
            out.extend_from_slice(&size_class_gib.to_be_bytes());
            out.extend_from_slice(&[*repair, exit_code.cast_unsigned()]);
            let bytes = path.as_bytes();
            // A path past the bound is cut here rather than shifting a field, and the decoder
            // refuses such a frame on its padding.
            let taken = bytes.len().min(MAX_VOLUME_PATH_BYTES);
            let mut field = [0_u8; MAX_VOLUME_PATH_BYTES];
            field[..taken].copy_from_slice(&bytes[..taken]);
            out.extend_from_slice(&u16::try_from(taken).map_or([0, 0], u16::to_be_bytes));
            out.extend_from_slice(&field);
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
        OPENED_FRAME_LEN if bytes[0] == 14 => {
            let length_at = OPENED_FRAME_LEN - MAX_VOLUME_PATH_BYTES - 2;
            let path_len = usize::from(u16::from_be_bytes(array(&bytes[length_at..length_at + 2])));
            let field = &bytes[OPENED_FRAME_LEN - MAX_VOLUME_PATH_BYTES..];
            if path_len == 0
                || path_len > MAX_VOLUME_PATH_BYTES
                || field[path_len..].iter().any(|byte| *byte != 0)
            {
                return Err(ProtocolError("volume path"));
            }
            Ok(Reply::VolumeOpened {
                volume: volume(&bytes[1..17])?,
                size_class_gib: u32::from_be_bytes(array(&bytes[17..21])),
                repair: bytes[21],
                exit_code: bytes[22].cast_signed(),
                path: std::str::from_utf8(&field[..path_len])
                    .map_err(|_| ProtocolError("volume path"))?
                    .to_owned(),
            })
        }
        _ => Err(ProtocolError("reply")),
    }
}
