use super::{
    OVERLAY_FEATURES, OVERLAY_VOLUME_LABEL, derive_overlay_hash_seed, derive_overlay_uuid,
};
use crate::generation::{
    erofs::format_uuid,
    error::{CompileError, CompileErrorKind, CompilePhase},
    process::ToolOutcome,
};

/// Requires a clean read-only `e2fsck`, the pinned superblock facts, and exactly the sterile
/// `lost+found`, `upper`, and `work` directories with nothing inside `upper` or `work`.
pub(super) fn verify_class(
    capacity: u64,
    check: &ToolOutcome,
    inspect: &[ToolOutcome],
) -> Result<(), CompileError> {
    if !check.succeeded()
        || inspect.len() != 4
        || inspect.iter().any(|outcome| !outcome.succeeded())
    {
        return Err(CompileError::new(
            CompilePhase::VerifyOverlay,
            CompileErrorKind::Toolchain,
        ));
    }
    let header = String::from_utf8_lossy(&inspect[0].stdout);
    let field = |name: &str| {
        header
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .map(str::trim)
            .ok_or_else(integrity)
    };
    let mut features: Vec<&str> = field("Filesystem features:")?.split(' ').collect();
    features.sort_unstable();
    let mut expected: Vec<&str> = OVERLAY_FEATURES.to_vec();
    expected.sort_unstable();
    let block_count: u64 = field("Block count:")?.parse().map_err(|_| integrity())?;
    if field("Filesystem UUID:")? != format_uuid(&derive_overlay_uuid(capacity))
        || field("Directory Hash Seed:")? != format_uuid(&derive_overlay_hash_seed(capacity))
        || field("Filesystem volume name:")? != OVERLAY_VOLUME_LABEL
        || field("Block size:")? != "4096"
        || field("Inode size:")? != "256"
        || field("Reserved block count:")? != "0"
        || block_count.checked_mul(4096) != Some(capacity)
        || features != expected
    {
        return Err(integrity());
    }
    require_listing(&inspect[1], &[".", "..", "lost+found", "upper", "work"])?;
    require_listing(&inspect[2], &[".", ".."])?;
    require_listing(&inspect[3], &[".", ".."])
}

fn require_listing(outcome: &ToolOutcome, expected: &[&str]) -> Result<(), CompileError> {
    let text = String::from_utf8_lossy(&outcome.stdout);
    let mut names: Vec<&str> = text
        .lines()
        .filter_map(|line| line.split_whitespace().last())
        .collect();
    names.sort_unstable();
    let mut expected = expected.to_vec();
    expected.sort_unstable();
    if names != expected {
        return Err(integrity());
    }
    Ok(())
}

const fn integrity() -> CompileError {
    CompileError::new(CompilePhase::VerifyOverlay, CompileErrorKind::Integrity)
}
