//! D1: the writeback cap in bytes.
//!
//! The kernel honours `dirty_bytes` only while both ratios are zero, and it flushes the dirty set
//! at the threshold rather than throttling writers, so a run that crosses the threshold pauses for
//! a write-out it did not ask for. Capping the set in bytes removes that pause on the large shape;
//! the ratios are zeroed first so the byte cap is the one that governs.

use super::{DIRTY_EXPIRE_CENTISECS, dirty_background, write_sysctl};

/// The complete writeback policy for a cap of `cap_bytes`, in the order it must be written.
///
/// Ratios come first because the kernel ignores a byte cap while a non-zero ratio sits beside it.
fn policy(cap_bytes: u64) -> [(&'static str, u64); 5] {
    [
        ("vm/dirty_ratio", 0),
        ("vm/dirty_background_ratio", 0),
        ("vm/dirty_bytes", cap_bytes),
        ("vm/dirty_background_bytes", dirty_background(cap_bytes)),
        ("vm/dirty_expire_centisecs", DIRTY_EXPIRE_CENTISECS),
    ]
}

/// Writes the complete writeback policy for a cap of `cap_bytes`.
///
/// # Errors
///
/// Returns a short reason for the first sysctl that could not be written.
pub(super) fn apply(cap_bytes: u64) -> Result<(), String> {
    for (name, value) in policy(cap_bytes) {
        write_sysctl(name, value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn the_block_zeroes_both_ratios_first_then_sets_the_byte_cap() {
        let block = policy(4 * GIB);
        assert_eq!(
            block,
            [
                ("vm/dirty_ratio", 0),
                ("vm/dirty_background_ratio", 0),
                ("vm/dirty_bytes", 4 * GIB),
                ("vm/dirty_background_bytes", 3 * GIB),
                ("vm/dirty_expire_centisecs", 360_000),
            ]
        );
    }
}
