//! The answer one command outcome gets, and the line between a refusal and an outage.

use soma::{BackendFailureKind, ManagedFailure, RunFailureKind, TerminalStatus};

use super::outcome::{exit_code, failure_error, run_failure_error};

#[test]
fn every_outcome_the_sandbox_survived_names_its_code_and_says_the_sandbox_is_alive() {
    // The engine keeps the sandbox behind every one of these, so an answer that reads as an
    // outage, or that leaves the caller to guess whether the sandbox is still there, would be
    // wrong twice over.
    for (kind, status, code) in [
        (RunFailureKind::TimedOut, 504, "EXEC_TIMEOUT"),
        (
            RunFailureKind::OutputLimitExceeded,
            413,
            "EXEC_OUTPUT_LIMIT",
        ),
        (
            RunFailureKind::SpawnFailed { errno: 2 },
            400,
            "EXEC_SPAWN_FAILED",
        ),
    ] {
        let error = run_failure_error(kind);
        assert_eq!((error.status, error.code), (status, code), "{kind:?}");
        assert!(!error.retryable, "{kind:?} must not invite a retry");
        assert_eq!(
            error.details.as_ref().expect("details")["sandbox_alive"],
            serde_json::json!(true),
            "{kind:?}"
        );
    }
}

#[test]
fn a_program_that_could_not_start_names_the_errno_the_guest_reported() {
    let error = run_failure_error(RunFailureKind::SpawnFailed { errno: 13 });
    assert_eq!(
        error.details.as_ref().expect("details")["errno"],
        serde_json::json!(13)
    );
}

#[test]
fn only_a_lost_agent_is_reported_as_an_agent_outage() {
    for kind in [
        BackendFailureKind::Unavailable,
        BackendFailureKind::IsolationFailure,
        BackendFailureKind::GuestFailure,
        BackendFailureKind::Timeout,
        BackendFailureKind::OutputLimit,
        BackendFailureKind::CleanupFailure,
    ] {
        let error = failure_error(&ManagedFailure::Backend(kind));
        assert_eq!(
            (error.status, error.code),
            (502, "AGENT_UNAVAILABLE"),
            "{kind:?}"
        );
    }

    // And the refusals keep their own codes rather than borrowing that one.
    for (kind, status, code) in [
        (
            BackendFailureKind::WorkloadRejected,
            400,
            "WORKLOAD_REJECTED",
        ),
        (
            BackendFailureKind::ResourceConflict,
            409,
            "RESOURCE_CONFLICT",
        ),
        (BackendFailureKind::Unsupported, 501, "BACKEND_UNSUPPORTED"),
    ] {
        let error = failure_error(&ManagedFailure::Backend(kind));
        assert_eq!((error.status, error.code), (status, code), "{kind:?}");
        assert!(!error.retryable, "{kind:?}");
    }
}

#[test]
fn a_signal_ended_command_reports_the_exit_code_a_shell_would() {
    assert_eq!(exit_code(TerminalStatus::Exited { code: 3 }), Some(3));
    assert_eq!(
        exit_code(TerminalStatus::Signaled { signal: Some(9) }),
        Some(137)
    );
    // A backend that could not name the signal still says the command was killed rather than
    // that it exited.
    assert_eq!(
        exit_code(TerminalStatus::Signaled { signal: None }),
        Some(128)
    );

    // A status that describes a command with no result carries no exit code at all.
    for status in [
        TerminalStatus::TimedOut,
        TerminalStatus::OutputLimitExceeded,
        TerminalStatus::Failed,
    ] {
        assert_eq!(exit_code(status), None, "{status:?}");
    }
}
