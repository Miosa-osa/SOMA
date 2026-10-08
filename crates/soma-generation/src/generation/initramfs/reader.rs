//! Reading an initramfs archive back, and refusing any deviation from the allowlist.
//!
//! The reader is the other half of [`super::build_initramfs`]: it accepts exactly the entries
//! that function writes, in the order it writes them, and nothing else. A layout v2 archive is
//! rejected because its `etc/soma/responder.key` entry is not in the v3 allowlist, so a
//! Generation carrying an immutable guest secret cannot be verified through here.

use soma_guest::CaptureWarmPlan;

use super::{
    CAPTURE_WARM_PATH, EARLY_INIT_PATH, Fields, GUEST_AGENT_PATH, HEADER_LEN, InitramfsContents,
    MAGIC, MAX_ENTRIES, TRAILER, TRAILER_FIELDS, fields, invalid, layout, layout_version,
};
use crate::generation::{artifacts::Sha256Digest, error::CompileError};

/// Decodes and verifies a layout v3 or v4 archive, rejecting any deviation from the allowlist.
///
/// A layout v2 archive is rejected because its `etc/soma/responder.key` entry is not in the
/// v3 allowlist, so a Generation carrying an immutable guest secret cannot be verified here.
/// A layout v4 archive is accepted only when its warm entry is the last one and holds a
/// canonical capture warm plan.
///
/// # Errors
///
/// Returns [`CompileErrorKind::InvalidInput`](super::super::error::CompileErrorKind::InvalidInput)
/// for malformed headers, ordering, padding, metadata, unknown paths, a non-canonical plan, or
/// trailing bytes.
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

struct RawEntry<'a> {
    fields: Fields,
    name: &'a [u8],
    body: &'a [u8],
    next: usize,
}

fn read_entry(archive: &[u8], start: usize) -> Result<RawEntry<'_>, CompileError> {
    let header = archive
        .get(start..start.checked_add(HEADER_LEN).ok_or_else(invalid)?)
        .ok_or_else(invalid)?;
    if &header[..6] != MAGIC {
        return Err(invalid());
    }
    let mut fields = [0_u32; 13];
    for (index, field) in fields.iter_mut().enumerate() {
        let text =
            std::str::from_utf8(&header[6 + index * 8..14 + index * 8]).map_err(|_| invalid())?;
        if !text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(invalid());
        }
        *field = u32::from_str_radix(text, 16).map_err(|_| invalid())?;
    }
    let name_size = usize::try_from(fields[11]).map_err(|_| invalid())?;
    let body_size = usize::try_from(fields[6]).map_err(|_| invalid())?;
    if name_size == 0 || name_size > 256 {
        return Err(invalid());
    }
    let name_start = start + HEADER_LEN;
    let name_end = name_start.checked_add(name_size).ok_or_else(invalid)?;
    let name = archive.get(name_start..name_end).ok_or_else(invalid)?;
    if name[name_size - 1] != 0 || name[..name_size - 1].contains(&0) {
        return Err(invalid());
    }
    let body_start = align4(name_end)?;
    require_zero(archive, name_end, body_start)?;
    let body_end = body_start.checked_add(body_size).ok_or_else(invalid)?;
    let body = archive.get(body_start..body_end).ok_or_else(invalid)?;
    let next = align4(body_end)?;
    require_zero(archive, body_end, next)?;
    Ok(RawEntry {
        fields,
        name: &name[..name_size - 1],
        body,
        next,
    })
}

fn require_zero(archive: &[u8], start: usize, end: usize) -> Result<(), CompileError> {
    let padding = archive.get(start..end).ok_or_else(invalid)?;
    if padding.iter().any(|byte| *byte != 0) {
        return Err(invalid());
    }
    Ok(())
}

fn align4(value: usize) -> Result<usize, CompileError> {
    value.checked_add(3).map(|sum| sum & !3).ok_or_else(invalid)
}
