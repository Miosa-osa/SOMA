use soma::CommandStatus;

use super::observed_output;

const ALLOWANCE: usize = 1024 * 1024;

/// SOMA#213: a command the guest stopped at its allowance kept exactly the allowance, and its
/// observation must say more was offered, or the engine reads the stop as contradictory evidence.
#[test]
fn a_command_stopped_at_its_allowance_observed_more_than_it_kept() {
    let output = observed_output(
        CommandStatus::OutputLimitExceeded,
        vec![b'a'; ALLOWANCE],
        Vec::new(),
    );
    assert_eq!(
        format!("{output:?}"),
        format!(
            "ObservedOutput {{ stdout_bytes: {ALLOWANCE}, stdout_observed_bytes: {}, \
             stderr_bytes: 0, stderr_observed_bytes: 0 }}",
            ALLOWANCE + 1
        )
    );
}

#[test]
fn a_stop_with_only_stderr_counts_the_extra_byte_on_stderr() {
    let output = observed_output(
        CommandStatus::OutputLimitExceeded,
        Vec::new(),
        vec![b'e'; ALLOWANCE],
    );
    assert!(format!("{output:?}").ends_with(&format!(
        "stderr_bytes: {ALLOWANCE}, stderr_observed_bytes: {} }}",
        ALLOWANCE + 1
    )));
}

#[test]
fn a_finished_command_observed_exactly_what_it_kept() {
    let output = observed_output(
        CommandStatus::Exited { code: 0 },
        b"out".to_vec(),
        b"err".to_vec(),
    );
    assert_eq!(
        format!("{output:?}"),
        "ObservedOutput { stdout_bytes: 3, stdout_observed_bytes: 3, stderr_bytes: 3, \
         stderr_observed_bytes: 3 }"
    );
}
