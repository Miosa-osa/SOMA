//! Decoding one archive and holding it to the allowlist.
//!
//! Verification never repairs: an archive whose entries, order, metadata, padding, or trailing
//! bytes differ from the allowlist is rejected. A layout v2 archive in particular is refused,
//! because its `etc/soma/responder.key` entry is not in the v3 allowlist and an immutable
//! Generation must carry no guest secret.

use soma_guest::CaptureWarmPlan;

use super::super::artifacts::Sha256Digest;
use super::{
    CAPTURE_WARM_PATH, CompileError, EARLY_INIT_PATH, GUEST_AGENT_PATH, InitramfsContents,
    MAX_ENTRIES, TRAILER, TRAILER_FIELDS, fields, invalid, layout, layout_version, read_entry,
};

/// Decodes and verifies a layout v3 or v4 archive, rejecting any deviation from the allowlist.
///
/// A layout v2 archive is rejected because its `etc/soma/responder.key` entry is not in the
/// v3 allowlist, so a Generation carrying an immutable guest secret cannot be verified here.
/// A layout v4 archive is accepted only when its warm entry is the last one and holds a
/// canonical capture warm plan.
///
/// # Errors
///
/// Returns `CompileErrorKind::InvalidInput` for malformed headers, ordering, padding,
/// metadata, unknown paths, a non-canonical plan, or trailing bytes.
pub fn verify_initramfs(archive: &[u8]) -> Result<InitramfsContents, CompileError> {
    let mut cursor = 0_usize;
    let mut expected = layout(true).enumerate();
    let mut early_init = None;
    let mut guest_agent = None;
    let mut capture_warm = None;
    for _ in 0..=MAX_ENTRIES {
        let entry = read_entry(archive, cursor)?;
        cursor = entry.next;
        if entry.name == TRAILER.as_bytes() {
            // Only the optional warm entry may remain unconsumed, and only when it is absent.
            let remaining = expected.next();
            if remaining.is_some_and(|(_, entry)| entry.0 != CAPTURE_WARM_PATH)
                || (remaining.is_some() && capture_warm.is_some())
                || entry.fields != TRAILER_FIELDS
            {
                return Err(invalid());
            }
            let trailing = archive.get(cursor..).ok_or_else(invalid)?;
            if trailing.len() >= 512 || trailing.iter().any(|byte| *byte != 0) {
                return Err(invalid());
            }
            return Ok(InitramfsContents {
                early_init_digest: early_init.ok_or_else(invalid)?,
                guest_agent_digest: guest_agent.ok_or_else(invalid)?,
                layout_version: layout_version(capture_warm.is_some()),
                capture_warm,
            });
        }
        let (index, (path, mode, rdev)) = expected.next().ok_or_else(invalid)?;
        let inode = u32::try_from(index + 1).map_err(|_| invalid())?;
        let size = u32::try_from(entry.body.len()).map_err(|_| invalid())?;
        if entry.name != path.as_bytes()
            || entry.fields != fields(inode, *mode, *rdev, size, path.len())
        {
            return Err(invalid());
        }
        match *path {
            EARLY_INIT_PATH => early_init = Some(Sha256Digest::of(entry.body)),
            GUEST_AGENT_PATH => guest_agent = Some(Sha256Digest::of(entry.body)),
            CAPTURE_WARM_PATH => {
                capture_warm = Some(CaptureWarmPlan::decode(entry.body).map_err(|_| invalid())?);
            }
            _ if !entry.body.is_empty() => return Err(invalid()),
            _ => {}
        }
    }
    Err(invalid())
}
