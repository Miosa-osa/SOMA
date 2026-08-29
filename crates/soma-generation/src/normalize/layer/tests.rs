use super::Budget;
use crate::{NormalizeErrorKind, RootfsLimits};

#[test]
fn checked_layer_counters_fail_closed_at_their_numeric_boundaries() {
    let limits = RootfsLimits {
        max_entries: u32::MAX,
        max_metadata_bytes: u64::MAX,
        max_content_bytes: u64::MAX,
        ..RootfsLimits::default()
    };
    for mut budget in [
        Budget {
            entries: u32::MAX,
            metadata_bytes: 0,
            content_bytes: 0,
        },
        Budget {
            entries: 0,
            metadata_bytes: u64::MAX,
            content_bytes: 0,
        },
        Budget {
            entries: 0,
            metadata_bytes: 0,
            content_bytes: u64::MAX,
        },
    ] {
        assert_eq!(
            budget.observe(b"x", None, 1, limits).unwrap_err().kind(),
            NormalizeErrorKind::LimitExceeded
        );
    }
}
