use crate::{ImportErrorKind, ImportPhase};

use super::{MAX_ENTRIES, MAX_PATH_METADATA_BYTES, ValidationBudget};

#[test]
fn entry_count_is_aggregated_across_layers() {
    let mut budget = ValidationBudget::new(MAX_ENTRIES, MAX_PATH_METADATA_BYTES);
    budget.observe(MAX_ENTRIES, 0).unwrap();

    assert_limit(budget.observe(1, 0));
}

#[test]
fn path_metadata_is_aggregated_across_layers() {
    let mut budget = ValidationBudget::new(MAX_ENTRIES, MAX_PATH_METADATA_BYTES);
    budget.observe(0, MAX_PATH_METADATA_BYTES).unwrap();

    assert_limit(budget.observe(0, 1));
}

fn assert_limit(result: Result<(), crate::ImportError>) {
    let error = result.unwrap_err();
    assert_eq!(error.phase(), ImportPhase::VerifyLayer);
    assert_eq!(error.kind(), ImportErrorKind::LimitExceeded);
}
