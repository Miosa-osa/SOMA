//! The drain after a deadline leaves the terminal report inside the host's delivery grace.

use std::time::{Duration, Instant};

use soma_guest::TerminalStatus;

use super::super::TIMEOUT_DRAIN;
use super::{command, run};

#[test]
fn a_pipe_held_outside_the_group_cannot_delay_the_report_past_the_delivery_grace() {
    // `setsid` moves the sleeper out of the command's group, so killing the group leaves it
    // holding the command's stdout. Before the drain after a deadline was bounded by
    // `TIMEOUT_DRAIN`, this waited the whole `KILL_GRACE` and the terminal report missed the
    // host's delivery deadline (deadline + `DELIVERY_GRACE`), which lost the Instance.
    let timeout = Duration::from_millis(200);
    let started = Instant::now();
    let (completion, _) = run(&command(
        "/bin/sh",
        &["-c", "(setsid /bin/sleep 3 &); /bin/sleep 30"],
        200,
        1,
    ));
    let elapsed = started.elapsed();

    assert_eq!(completion.status, TerminalStatus::TimedOut);
    assert!(
        elapsed < timeout + crate::lifecycle::DELIVERY_GRACE / 2,
        "the report must leave room inside the delivery grace, took {elapsed:?}"
    );
    assert!(TIMEOUT_DRAIN < crate::lifecycle::DELIVERY_GRACE / 2);
}
