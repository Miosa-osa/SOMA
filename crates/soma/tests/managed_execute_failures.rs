//! Which execute failures cost a sandbox and which do not.
//!
//! Every test here is the same shape: something goes wrong, and then an ordinary command runs on
//! the same sandbox. The second command is the assertion. A failure the guest agent reported, or
//! a request the runtime would not take, leaves a machine that answers the next command; a
//! failure nobody observed releases the machine, and the next command then reads the state the
//! release wrote.

mod support;

use std::sync::{Arc, Mutex};

use soma::{
    BackendFailureKind, CleanupDisposition, CleanupMethod, DirectCommand, Engine,
    ExecuteMachineRequest, ExecutionLimits, FailurePhase, InstanceId, LaunchMachineRequest,
    MachineShape, ManagedFailure, OciImage, OperationId, RunFailure, RunFailureKind,
    TerminalStatus,
};
use support::{Mode, TestBackend};

/// A launched sandbox, and the log of what the backend was asked to do.
struct Sandbox {
    engine: Engine<TestBackend>,
    calls: Arc<Mutex<Vec<&'static str>>>,
    instance: InstanceId,
}

fn launched(mode: Mode) -> Sandbox {
    let (backend, calls) = TestBackend::new(mode);
    let mut engine = Engine::new(backend);
    let instance = InstanceId::new("22222222222222222222222222222222").expect("instance");
    engine
        .launch_machine(LaunchMachineRequest::new(
            operation_id('1'),
            instance.clone(),
            OciImage::parse("node:22").expect("image"),
            MachineShape::new(1, 1_024, 8_192).expect("shape"),
        ))
        .expect("launch");
    Sandbox {
        engine,
        calls,
        instance,
    }
}

impl Sandbox {
    fn command(&mut self, digit: char) -> Result<soma::MachineExecution, ManagedFailure> {
        self.command_with_limit(digit, 16 * 1024 * 1024)
    }

    fn command_with_limit(
        &mut self,
        digit: char,
        max_output_bytes: u64,
    ) -> Result<soma::MachineExecution, ManagedFailure> {
        self.engine.execute_machine(ExecuteMachineRequest::new(
            operation_id(digit),
            self.instance.clone(),
            DirectCommand::new("/usr/local/bin/node", ["--version"]).expect("command"),
            ExecutionLimits::new(30_000, max_output_bytes).expect("limits"),
        ))
    }

    /// Runs one command and returns the operation failure it must be.
    fn refused(&mut self, digit: char, max_output_bytes: u64) -> RunFailure {
        run_failure(
            self.command_with_limit(digit, max_output_bytes)
                .expect_err("the command fails"),
        )
    }

    fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().expect("call log poisoned").clone()
    }

    /// The property every failure here is about: an ordinary command still runs. A machine that
    /// was released answers this one with a state failure instead.
    fn still_serves_commands(&mut self) {
        self.command('9').expect("the sandbox survived the failure");
        assert!(
            !self.calls().contains(&"cleanup"),
            "a kept sandbox is never released: {:?}",
            self.calls()
        );
    }
}

#[test]
fn a_command_the_runtime_would_not_take_leaves_the_sandbox_serving() {
    let mut sandbox = launched(Mode::WorkloadRejected);

    let failure = sandbox.refused('4', 16 * 1024 * 1024);

    assert_eq!(
        failure.kind(),
        RunFailureKind::Backend {
            phase: FailurePhase::Command,
            kind: BackendFailureKind::WorkloadRejected,
        }
    );
    assert_eq!(sandbox.calls(), ["resolve", "launch", "execute"]);
    sandbox.still_serves_commands();
}

#[test]
fn a_program_the_guest_cannot_start_leaves_the_sandbox_serving() {
    let mut sandbox = launched(Mode::SpawnFailed);

    // The absolute path does not exist, which is the one way a tenant reached this: a bare name
    // is resolved through `/usr/bin/env` and exits 127 instead.
    let failure = sandbox.refused('4', 16 * 1024 * 1024);

    assert_eq!(failure.kind(), RunFailureKind::SpawnFailed { errno: 2 });
    assert_eq!(
        failure.receipt().cleanup().method(),
        CleanupMethod::NotApplicable,
        "a machine that was kept released nothing"
    );
    sandbox.still_serves_commands();
}

#[test]
fn a_command_the_guest_stopped_at_its_deadline_leaves_the_sandbox_serving() {
    let mut sandbox = launched(Mode::Timeout);

    let failure = sandbox.refused('4', 16 * 1024 * 1024);

    assert_eq!(failure.kind(), RunFailureKind::TimedOut);
    assert_eq!(
        failure.receipt().terminal_status(),
        &TerminalStatus::TimedOut
    );
    assert_eq!(
        failure
            .output()
            .expect("the output the command did produce is kept")
            .stdout(),
        b"v22.23.2\n"
    );
    sandbox.still_serves_commands();
}

#[test]
fn a_command_that_exhausted_its_output_allowance_leaves_the_sandbox_serving() {
    let mut sandbox = launched(Mode::OutputLimit);

    let failure = sandbox.refused('4', 10);

    assert_eq!(failure.kind(), RunFailureKind::OutputLimitExceeded);
    assert_eq!(
        failure.receipt().terminal_status(),
        &TerminalStatus::OutputLimitExceeded
    );
    sandbox.still_serves_commands();
}

#[test]
fn the_receipt_of_a_kept_sandbox_is_valid_and_says_nothing_was_released() {
    for (mode, digit) in [
        (Mode::WorkloadRejected, '4'),
        (Mode::SpawnFailed, '5'),
        (Mode::Timeout, '6'),
        (Mode::OutputLimit, '7'),
    ] {
        let mut sandbox = launched(mode);
        let failure = sandbox.refused(digit, 10);

        let cleanup = failure.receipt().cleanup();
        assert_eq!(cleanup.method(), CleanupMethod::NotApplicable, "{mode:?}");
        assert_eq!(cleanup.machine(), CleanupDisposition::NotOwned, "{mode:?}");
        failure
            .receipt()
            .validate()
            .unwrap_or_else(|_| panic!("{mode:?} produced an invalid receipt"));
    }
}

#[test]
fn a_signal_ended_command_is_a_completed_command_and_not_a_lost_sandbox() {
    let mut sandbox = launched(Mode::Signaled);

    let executed = sandbox
        .command('4')
        .expect("a command the guest reaped is a result, not an outage");

    assert_eq!(
        executed.receipt().terminal_status(),
        &TerminalStatus::Signaled { signal: None }
    );
    // The result is remembered, so the same command is not run a second time.
    let replay = sandbox
        .command('4')
        .expect_err("the operation was completed");
    assert!(matches!(replay, ManagedFailure::ReplayUnavailable(_)));
    sandbox.command('9').expect("the sandbox is still there");
}

#[test]
fn an_unobserved_failure_still_ends_the_sandbox() {
    let mut sandbox = launched(Mode::CommandFailure);

    let failure = sandbox.refused('4', 16 * 1024 * 1024);

    assert_eq!(
        failure.kind(),
        RunFailureKind::Backend {
            phase: FailurePhase::Command,
            kind: BackendFailureKind::GuestFailure,
        }
    );
    assert!(failure.receipt().cleanup().is_complete());
    assert_eq!(
        sandbox.calls(),
        ["resolve", "launch", "execute", "cleanup"],
        "a machine nobody can account for is released"
    );
    let stopped = sandbox.command('9').expect_err("the machine ended");
    assert_eq!(
        stopped,
        ManagedFailure::State(soma::ManagedStateError::MachineStopped)
    );
}

/// The operation failure a managed call reports, or a panic naming what it reported instead.
fn run_failure(failure: ManagedFailure) -> RunFailure {
    match failure {
        ManagedFailure::Operation(failure) => *failure,
        other => panic!("expected an operation failure, got {other:?}"),
    }
}

fn operation_id(digit: char) -> OperationId {
    OperationId::new(digit.to_string().repeat(32)).expect("operation")
}
