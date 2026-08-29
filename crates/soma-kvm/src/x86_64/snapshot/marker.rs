//! The capture-point marker section.
//!
//! The marker is the snapshot's own statement of where it was taken. Version 1 certifies one
//! capture point: the disconnected repair wait the pinned guest agent reaches before any
//! launch page exists, so the memory image can carry no Instance identity, session key, or
//! network authority. Restore refuses a snapshot that does not say this.
//!
//! Layout, big-endian and fixed order: a 20-byte tag, a one-byte capture-point kind, a
//! two-byte length, and exactly that many bytes of the console line the agent printed.

use super::error::SnapshotError;

/// Fixed tag of the marker payload.
const TAG: &[u8; 20] = b"SOMA-REPAIR-POINT-V1";
/// The only capture point version 1 certifies.
const PRE_LAUNCH_REPAIR_WAIT: u8 = 1;
/// Upper bound on the recorded console line.
const MAX_LINE: usize = 256;
const HEADER: usize = TAG.len() + 1 + 2;

/// Encodes the marker for the console line the agent printed at the capture point.
#[must_use]
pub(super) fn encode(line: &[u8]) -> Vec<u8> {
    let line = &line[..line.len().min(MAX_LINE)];
    let mut payload = Vec::with_capacity(HEADER + line.len());
    payload.extend_from_slice(TAG);
    payload.push(PRE_LAUNCH_REPAIR_WAIT);
    payload.extend_from_slice(&u16::try_from(line.len()).unwrap_or(0).to_be_bytes());
    payload.extend_from_slice(line);
    payload
}

/// Decodes the marker and requires it to describe the pre-launch repair wait.
///
/// # Errors
///
/// Returns [`SnapshotError::RepairPointMarker`] for any other tag, kind, length, or shape.
pub(super) fn decode(bytes: &[u8]) -> Result<Vec<u8>, SnapshotError> {
    let reject = || SnapshotError::RepairPointMarker;
    let tag = bytes.get(..TAG.len()).ok_or_else(reject)?;
    if tag != TAG || bytes.get(TAG.len()).copied() != Some(PRE_LAUNCH_REPAIR_WAIT) {
        return Err(reject());
    }
    let length = bytes
        .get(TAG.len() + 1..HEADER)
        .and_then(|raw| <[u8; 2]>::try_from(raw).ok())
        .map(u16::from_be_bytes)
        .map(usize::from)
        .ok_or_else(reject)?;
    if length > MAX_LINE || bytes.len() != HEADER + length {
        return Err(reject());
    }
    Ok(bytes[HEADER..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::{HEADER, MAX_LINE, PRE_LAUNCH_REPAIR_WAIT, TAG, decode, encode};
    use crate::x86_64::snapshot::error::SnapshotError;

    #[test]
    fn a_marker_round_trips_and_every_other_shape_is_refused() {
        let payload = encode(b"soma-guest-agent: awaiting launch material");
        assert_eq!(&payload[..TAG.len()], TAG);
        assert_eq!(
            decode(&payload).unwrap(),
            b"soma-guest-agent: awaiting launch material".to_vec()
        );

        for truncated in 0..payload.len() {
            assert_eq!(
                decode(&payload[..truncated]),
                Err(SnapshotError::RepairPointMarker),
                "prefix of {truncated} bytes was accepted"
            );
        }
        let mut trailing = payload.clone();
        trailing.push(0);
        assert_eq!(decode(&trailing), Err(SnapshotError::RepairPointMarker));

        let mut wrong_kind = payload.clone();
        wrong_kind[TAG.len()] = PRE_LAUNCH_REPAIR_WAIT + 1;
        assert_eq!(decode(&wrong_kind), Err(SnapshotError::RepairPointMarker));

        let mut wrong_tag = payload;
        wrong_tag[0] ^= 1;
        assert_eq!(decode(&wrong_tag), Err(SnapshotError::RepairPointMarker));
    }

    #[test]
    fn an_oversized_line_is_bounded_at_the_capture_point() {
        let payload = encode(&vec![b'x'; MAX_LINE * 4]);
        assert_eq!(payload.len(), HEADER + MAX_LINE);
        assert_eq!(decode(&payload).unwrap().len(), MAX_LINE);
        assert_eq!(decode(&encode(b"")).unwrap(), Vec::<u8>::new());
    }
}
