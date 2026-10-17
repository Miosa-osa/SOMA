//! The create `Server-Timing` header: every segment is named, and the launch
//! segments are read from the receipt's own milestones.

use std::time::Duration;

use soma::{Observation, ObservationUnavailable, PreparationClass};

use super::{LaunchPhases, Timing};
use crate::runner::backend::CallTiming;
use crate::runner::journal::LaunchPreparation;

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
        wait: None,
        run: None,
        launch_preparation: None,
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
fn the_two_segments_a_call_earns_are_appended_and_no_others_are() {
    let bare = Timing::default();
    assert_eq!(bare.header().split(',').count(), 10);
    assert!(
        !bare.header().contains("wait;") && !bare.header().contains("run;"),
        "a call that neither waited nor ran a command reports neither: {}",
        bare.header()
    );

    // A call that waited for a launch its create had answered early for reports how long, as a
    // segment of its own at the end.
    let waited = Timing {
        wait: Some(Duration::from_micros(7_736)),
        ..bare
    };
    assert!(
        waited.header().ends_with(",wait;dur=7.736"),
        "{}",
        waited.header()
    );
    assert_eq!(waited.header().split(',').count(), 11);

    // A create-and-run reports the command's own part of its facade call, and a create-and-run
    // never waited: it is the create that waited.
    let ran = Timing {
        run: Some(Duration::from_millis(13)),
        ..bare
    };
    assert!(
        ran.header().ends_with(",run;dur=13.000"),
        "{}",
        ran.header()
    );
    assert!(!ran.header().contains("wait;"), "{}", ran.header());

    // Both together keep the wait first, which is the order they happen in.
    let both = Timing {
        wait: Some(Duration::from_millis(1)),
        run: Some(Duration::from_millis(2)),
        ..bare
    };
    assert!(
        both.header().ends_with(",wait;dur=1.000,run;dur=2.000"),
        "{}",
        both.header()
    );
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

#[test]
fn server_timing_names_the_launch_path_alongside_the_segments() {
    let timing = Timing {
        auth: Duration::from_micros(12),
        call: CallTiming {
            pool: Duration::from_micros(1_500),
            exec: Duration::from_millis(21),
        },
        prep: Duration::from_micros(340),
        launch: LaunchPhases {
            assign: Duration::from_millis(13),
            ready: Duration::from_millis(4),
            ..LaunchPhases::default()
        },
        finish: Duration::from_micros(120),
        launch_preparation: Some(LaunchPreparation::OnDemand),
    };

    let header = timing.header();
    // A metric with a description and no duration: a fact about the request rather than a span
    // of it, so it cannot be read as one of the segments and cannot disturb them.
    assert!(header.ends_with(",launch;desc=\"on_demand\""), "{header}");
    assert!(!header.contains("launch;dur"), "{header}");
    assert!(
        header.starts_with("auth;dur=0.012,pool;dur=1.500,exec;dur=21.000,"),
        "{header}"
    );
}

#[test]
fn a_launch_with_no_reported_path_says_nothing_rather_than_guessing() {
    let timing = Timing {
        launch_preparation: None,
        ..Timing::default()
    };
    assert!(!timing.header().contains("launch;"), "{}", timing.header());
}

#[test]
fn the_engines_own_classes_are_read_into_the_reported_names() {
    for (class, expected) in [
        (
            PreparationClass::PreparedWorker,
            LaunchPreparation::Prepared,
        ),
        (PreparationClass::OnDemand, LaunchPreparation::OnDemand),
        (
            PreparationClass::PausedLease,
            LaunchPreparation::PausedLease,
        ),
        (PreparationClass::ReadyLease, LaunchPreparation::ReadyLease),
    ] {
        assert_eq!(
            LaunchPreparation::of(&Observation::Observed(class)),
            Some(expected)
        );
    }
    // A receipt that never reached a launch reports no path at all, and the header must not
    // invent one.
    assert_eq!(
        LaunchPreparation::of(&Observation::Unavailable(
            ObservationUnavailable::NotReached
        )),
        None
    );
    assert_eq!(LaunchPreparation::Prepared.label(), "prepared");
    assert_eq!(LaunchPreparation::OnDemand.label(), "on_demand");
}
