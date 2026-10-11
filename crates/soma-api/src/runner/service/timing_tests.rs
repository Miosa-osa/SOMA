//! The create `Server-Timing` header: every segment is named, and the launch
//! segments are read from the receipt's own milestones.

use std::time::Duration;

use super::{LaunchPhases, Timing};
use crate::runner::backend::CallTiming;

/// The retained launch receipt the fixtures carry, milestones and all.
const RECEIPT: &str = include_str!("../../../tests/fixtures/receipt.json");

#[test]
fn server_timing_names_every_segment_of_a_create() {
    let timing = Timing {
        auth: Duration::from_micros(12),
        call: CallTiming {
            pool: Duration::from_micros(1_500),
            exec: Duration::from_millis(21),
        },
        prep: Duration::from_micros(340),
        launch: LaunchPhases {
            resolve: Duration::from_micros(101),
            admit: Duration::from_micros(4_897),
            assign: Duration::from_millis(305),
            ready: Duration::from_millis(340),
            commit: Duration::from_millis(2),
        },
        finish: Duration::from_micros(120),
    };

    let header = timing.header();

    // The three segments the header always carried keep their names and order, so
    // a client reading `exec` today is unaffected.
    assert!(
        header.starts_with("auth;dur=0.012,pool;dur=1.500,exec;dur=21.000,"),
        "{header}"
    );
    for (name, expected) in [
        ("prep", "0.340"),
        ("resolve", "0.101"),
        ("admit", "4.897"),
        ("assign", "305.000"),
        ("ready", "340.000"),
        ("commit", "2.000"),
        ("finish", "0.120"),
    ] {
        assert!(
            header.contains(&format!("{name};dur={expected}")),
            "{name} missing from {header}"
        );
    }
    assert_eq!(header.split(',').count(), 10, "{header}");
}

#[test]
fn the_launch_segments_are_read_from_the_receipt_milestones() {
    let receipt: soma::ExecutionReceipt =
        serde_json::from_str(RECEIPT).expect("the retained receipt is valid");
    // The fixture's own milestones: accepted 0 ns, workload_resolved 101 ns,
    // admitted 4_998 ns, machine_launched 305_978_116 ns, ready 646_002_667 ns.
    let phases = LaunchPhases::from_receipt(&receipt, Duration::from_millis(900));

    assert_eq!(phases.resolve, Duration::from_nanos(101));
    assert_eq!(phases.admit, Duration::from_nanos(4_897));
    assert_eq!(phases.assign, Duration::from_nanos(305_973_118));
    assert_eq!(phases.ready, Duration::from_nanos(340_024_551));
    // `commit` is the remainder of the facade call after `ready`, so the segments
    // plus it reconstruct exactly the call the `exec` segment reports.
    assert_eq!(
        phases.resolve + phases.admit + phases.assign + phases.ready + phases.commit,
        Duration::from_millis(900)
    );
}

#[test]
fn a_ready_milestone_past_the_call_leaves_commit_at_zero() {
    // `commit` is derived by subtraction against the facade call's own duration,
    // which is a different clock from the milestone's; a disagreement must not
    // produce a segment larger than the call it belongs to.
    let receipt: soma::ExecutionReceipt =
        serde_json::from_str(RECEIPT).expect("the retained receipt is valid");
    let phases = LaunchPhases::from_receipt(&receipt, Duration::from_millis(1));

    assert_eq!(phases.commit, Duration::ZERO);
}
