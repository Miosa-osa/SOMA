//! Teardown must be total: one failing step may not abandon the resources after it.

use std::os::fd::OwnedFd;

use super::{StepResult, teardown};
use crate::{BundleNames, ConntrackZone};

#[test]
fn a_failing_step_does_not_abandon_the_steps_after_it() {
    let root = tempfile::tempdir().expect("state dir");
    let pin = root.path().join("not-a-namespace");
    std::fs::write(&pin, b"this is not a network namespace").expect("plant the pin");
    let names = BundleNames::new("deadbeef");
    let zone = ConntrackZone::new(9).expect("zone");

    let tap: OwnedFd = std::fs::File::open("/dev/null")
        .expect("a stand-in descriptor")
        .into();

    let evidence = teardown(&names, zone, &pin, Some(tap), Vec::new());

    assert_eq!(
        evidence.forwarding,
        StepResult::Failed,
        "a pin that is not a namespace cannot be entered"
    );
    assert!(
        evidence.failure.is_some(),
        "the first failure must be reported"
    );
    assert!(!evidence.complete);
    assert_eq!(
        evidence.tap,
        StepResult::Removed,
        "every later step must still run after an earlier step failed"
    );
    assert!(
        !evidence.ledger,
        "an incomplete release must write no ledger release record"
    );
}
