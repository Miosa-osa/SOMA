//! Building the deterministic `newc` archive from verified inputs.
//!
//! The archive is built to one exact shape so that its digest is a function of the inputs
//! alone. Nothing here decides the layout: the allowlist, the field encoder, and the trailer
//! all come from the parent module, and this only walks them in order.

use soma_guest::CaptureWarmPlan;

use super::{
    CAPTURE_WARM_PATH, CompileError, EARLY_INIT_PATH, GUEST_AGENT_PATH, TRAILER, TRAILER_FIELDS,
    build_limit, fields, layout, pad, push_entry,
};

/// Builds the deterministic `newc` archive: layout v3, or layout v4 when a capture warm plan
/// is declared.
///
/// Entries are emitted in raw path-byte order with root ownership, fixed modes, zero mtime,
/// sequential inode numbers, zero device numbers except the two character nodes, zero
/// padding, and a final `TRAILER!!!`.
///
/// # Errors
///
/// Returns `CompileErrorKind::LimitExceeded` when the total exceeds `max_bytes`.
pub fn build_initramfs(
    early_init: &[u8],
    guest_agent: &[u8],
    capture_warm: Option<&CaptureWarmPlan>,
    max_bytes: u64,
) -> Result<Vec<u8>, CompileError> {
    let warm = capture_warm.map(CaptureWarmPlan::encode);
    let mut archive = Vec::new();
    for (index, (path, mode, rdev)) in layout(warm.is_some()).enumerate() {
        let body: &[u8] = match *path {
            EARLY_INIT_PATH => early_init,
            GUEST_AGENT_PATH => guest_agent,
            CAPTURE_WARM_PATH => warm.as_deref().unwrap_or_default(),
            _ => &[],
        };
        let inode = u32::try_from(index + 1).map_err(|_| build_limit())?;
        let size = u32::try_from(body.len()).map_err(|_| build_limit())?;
        push_entry(
            &mut archive,
            &fields(inode, *mode, *rdev, size, path.len()),
            path.as_bytes(),
            body,
        );
    }
    push_entry(&mut archive, &TRAILER_FIELDS, TRAILER.as_bytes(), &[]);
    pad(&mut archive, 512);
    if u64::try_from(archive.len()).map_err(|_| build_limit())? > max_bytes {
        return Err(build_limit());
    }
    Ok(archive)
}
