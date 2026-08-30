//! Hostile-output gates: unbounded producers, sink disconnects, and the next operation.

use std::time::{Duration, Instant};

use soma_guest::TerminalStatus;

use super::super::{RESIDENT_OUTPUT_BYTES, execute};
use super::{
    HOSTILE_ALLOWANCE, HOSTILE_BOTH_PIPES, MAX_RESIDENT_GROWTH_BYTES, SlowCountingSink, command,
    group_is_gone, resident_high_water, run,
};

#[test]
fn hostile_output_on_both_pipes_stays_within_a_declared_resident_bound() {
    let before = resident_high_water();
    let mut sink = SlowCountingSink::new(Duration::from_millis(1));
    let started = Instant::now();
    let completion = execute(
        &command(
            "/bin/sh",
            &["-c", HOSTILE_BOTH_PIPES],
            60_000,
            HOSTILE_ALLOWANCE,
        ),
        &mut sink,
    )
    .expect("executor");
    let elapsed = started.elapsed();
    let growth = resident_high_water().saturating_sub(before);

    assert_eq!(completion.status, TerminalStatus::OutputLimit);
    assert_eq!(
        completion.stdout_bytes + completion.stderr_bytes,
        HOSTILE_ALLOWANCE,
        "the limit report must equal the exact delivered accounting"
    );
    assert_eq!(completion.stdout_bytes, sink.stdout);
    assert_eq!(completion.stderr_bytes, sink.stderr);
    // Neither pipe may spend the whole allowance: each pass gives every readable stream a
    // share, so a fast writer on one pipe cannot starve the other out of the record.
    assert!(
        completion.stdout_bytes > 0 && completion.stderr_bytes > 0,
        "both hostile pipes must deliver bytes: stdout {} stderr {}",
        completion.stdout_bytes,
        completion.stderr_bytes
    );
    assert!(
        sink.stdout > 0 && sink.stderr > 0,
        "both pipes must have competed: stdout={} stderr={}",
        sink.stdout,
        sink.stderr
    );
    assert!(
        sink.largest_chunk <= RESIDENT_OUTPUT_BYTES,
        "chunk {} exceeded the fixed buffer",
        sink.largest_chunk
    );
    assert!(
        growth <= MAX_RESIDENT_GROWTH_BYTES,
        "resident growth {growth} exceeded the declared bound {MAX_RESIDENT_GROWTH_BYTES} while \
         a hostile child wrote without end"
    );
    assert!(
        elapsed < Duration::from_secs(60),
        "completion was unbounded"
    );
    assert!(group_is_gone(completion.process_group));
}

#[test]
fn the_agent_accepts_the_next_lifecycle_operation_after_a_hostile_command() {
    let mut hostile = SlowCountingSink::new(Duration::from_micros(50));
    let first = execute(
        &command("/bin/sh", &["-c", HOSTILE_BOTH_PIPES], 60_000, 1 << 20),
        &mut hostile,
    )
    .expect("executor");
    assert_eq!(first.status, TerminalStatus::OutputLimit);

    let (next, sink) = run(&command("/bin/echo", &["-n", "alive"], 5_000, 64));

    assert_eq!(next.status, TerminalStatus::Exited(0));
    assert_eq!(sink.stdout, b"alive");
    assert!(group_is_gone(first.process_group));
    assert!(group_is_gone(next.process_group));
}
