//! Feed sequence numbers: they only increase, except at a snapshot, which is a reset point.

use std::time::Instant;

use super::{
    FeedViolation,
    tests::{enabled_table, event},
};

#[test]
fn a_snapshot_opening_a_connection_may_restart_the_sequence() {
    let table = enabled_table();
    table.begin_connection();
    let now = Instant::now();
    table
        .apply(&event(r#"{"seq":1,"kind":"snapshot_begin"}"#), now)
        .expect("a restarted sequence opens with a snapshot");
    table
        .apply(&event(r#"{"seq":2,"kind":"snapshot_end"}"#), now)
        .expect("applies");

    assert_eq!(table.last_seq(), 2);
    table.begin_connection();
    assert!(
        table
            .apply(&event(r#"{"seq":1,"kind":"heartbeat"}"#), now)
            .is_err(),
        "only a snapshot may restart the sequence"
    );
}

/// The live finding of 10-03: a control plane that resends a snapshot mid-stream with a lower
/// sequence costs no reconnect.
#[test]
fn a_snapshot_mid_stream_may_restart_the_sequence() {
    let table = enabled_table();
    let now = Instant::now();
    table
        .apply(&event(r#"{"seq":2,"kind":"snapshot_begin"}"#), now)
        .expect("a snapshot is a reset point, connection or not");
    table
        .apply(&event(r#"{"seq":3,"kind":"snapshot_end"}"#), now)
        .expect("applies");
    assert_eq!(table.last_seq(), 3);
    assert!(
        table
            .apply(&event(r#"{"seq":2,"kind":"heartbeat"}"#), now)
            .is_err(),
        "only a snapshot may go back"
    );
}

#[test]
fn sequence_numbers_must_increase() {
    let table = enabled_table();

    assert_eq!(
        table.apply(&event(r#"{"seq":4,"kind":"heartbeat"}"#), Instant::now()),
        Err(FeedViolation::SequenceRegressed {
            last: 4,
            received: 4
        })
    );
}
