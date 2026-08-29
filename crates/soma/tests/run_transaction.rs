mod support;

use soma::{Engine, TerminalStatus};
use support::{Mode, TestBackend, run_request};

#[test]
fn one_shot_run_resolves_launches_executes_and_cleans_in_order() {
    let (backend, calls) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::new(backend);

    let outcome = engine.run(run_request()).expect("run succeeds");

    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "execute", "cleanup"]
    );
    assert_eq!(
        outcome.receipt().terminal_status(),
        &TerminalStatus::Exited { code: 0 }
    );
    assert!(outcome.receipt().cleanup().is_complete());
    assert_eq!(outcome.output().stdout(), b"v22.23.2\n");
    assert_eq!(outcome.output().stderr(), b"");
}

#[test]
fn output_is_binary_safe_and_signal_termination_remains_distinct() {
    let (binary_backend, _) = TestBackend::new(Mode::BinaryOutput);
    let (signal_backend, _) = TestBackend::new(Mode::Signaled);
    let mut binary_engine = Engine::new(binary_backend);
    let mut signal_engine = Engine::new(signal_backend);

    let binary = binary_engine.run(run_request()).expect("binary run");
    let signaled = signal_engine.run(run_request()).expect("signaled run");

    assert_eq!(binary.output().stdout(), [0, 0xff, b'\n']);
    assert_eq!(binary.output().stderr(), [0x80]);
    assert_eq!(
        signaled.receipt().terminal_status(),
        &TerminalStatus::Signaled { signal: None }
    );
}
