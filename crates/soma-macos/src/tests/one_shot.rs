use crate::{
    BackendError, CleanupState, CommandFailure, CommandFailureReason, ExecutionStatus,
    NetworkPolicy, Operation, ProcessFailureKind,
};

use super::fixtures::{
    INSTANCE, backend, instance, one_shot, output, owned_inspection, strings, success,
};

#[test]
fn one_shot_runs_node_with_exact_resources_and_proves_cleanup() {
    let (backend, runner) = backend([
        Ok(success(b"v22.20.0\n".to_vec())),
        Ok(owned_inspection()),
        Ok(success(Vec::<u8>::new())),
    ]);

    let result = backend.run(&one_shot(65_536)).expect("one-shot succeeds");

    assert_eq!(result.status(), ExecutionStatus::Exited { code: 0 });
    assert_eq!(result.stdout(), b"v22.20.0\n");
    assert_eq!(result.cleanup(), Some(CleanupState::Complete));
    let resources = result.resources().expect("inspect exposes resources");
    assert_eq!(resources.vcpus(), 1);
    assert_eq!(resources.memory_bytes(), 1_073_741_824);
    let calls = runner.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(
        calls[0].arguments,
        strings(&[
            "run",
            "--name",
            &format!("soma-{INSTANCE}"),
            "--label",
            &format!("io.miosa.soma.instance={INSTANCE}"),
            "--cpus",
            "1",
            "--memory",
            "1024M",
            "--progress",
            "none",
            "--entrypoint",
            "/usr/local/bin/node",
            "node:22",
            "--version",
        ])
    );
    assert_eq!(
        calls[1].arguments,
        strings(&["inspect", &format!("soma-{INSTANCE}")])
    );
    assert_eq!(
        calls[2].arguments,
        strings(&["delete", "--force", &format!("soma-{INSTANCE}")])
    );
}

#[test]
fn one_shot_encodes_denied_network_policy_as_none() {
    let (backend, runner) = backend([
        Ok(success(Vec::<u8>::new())),
        Ok(owned_inspection()),
        Ok(success(Vec::<u8>::new())),
    ]);

    backend
        .run(&one_shot(65_536).with_network_policy(NetworkPolicy::Denied))
        .expect("one-shot succeeds");

    let arguments = &runner.calls()[0].arguments;
    let position = arguments
        .iter()
        .position(|argument| argument == "--network")
        .expect("denied policy has a network flag");
    assert_eq!(
        arguments.get(position + 1).map(String::as_str),
        Some("none")
    );
}

#[test]
fn one_shot_cleans_up_after_nonzero_guest_exit() {
    let (backend, runner) = backend([
        Ok(output(
            ExecutionStatus::Exited { code: 7 },
            Vec::new(),
            b"failed\n".to_vec(),
        )),
        Ok(owned_inspection()),
        Ok(success(Vec::<u8>::new())),
    ]);

    let result = backend
        .run(&one_shot(65_536))
        .expect("guest failure remains an explicit result");

    assert_eq!(result.status(), ExecutionStatus::Exited { code: 7 });
    assert_eq!(result.cleanup(), Some(CleanupState::Complete));
    assert_eq!(runner.calls().len(), 3);
}

#[test]
fn one_shot_cleans_up_after_timeout() {
    let (backend, runner) = backend([
        Ok(output(
            ExecutionStatus::TimedOut,
            b"partial".to_vec(),
            Vec::new(),
        )),
        Ok(owned_inspection()),
        Ok(success(Vec::<u8>::new())),
    ]);

    let result = backend
        .run(&one_shot(65_536))
        .expect("timeout is explicit after cleanup");

    assert_eq!(result.status(), ExecutionStatus::TimedOut);
    assert_eq!(result.cleanup(), Some(CleanupState::Complete));
    assert_eq!(runner.calls().len(), 3);
}

#[test]
fn one_shot_cleans_up_after_process_failure() {
    let (backend, runner) = backend([
        Err(ProcessFailureKind::SpawnFailed),
        Ok(owned_inspection()),
        Ok(success(Vec::<u8>::new())),
    ]);

    let failure = backend
        .run(&one_shot(65_536))
        .expect_err("process failure cannot be presented as execution");

    assert_eq!(
        failure,
        BackendError::Command {
            failure: CommandFailure::new(
                Operation::Run,
                CommandFailureReason::Process(ProcessFailureKind::SpawnFailed),
            ),
        }
    );
    assert_eq!(runner.calls().len(), 3);
}

#[test]
fn one_shot_fails_when_cleanup_cannot_be_proven() {
    let (backend, _) = backend([
        Ok(success(b"v22.20.0\n".to_vec())),
        Ok(owned_inspection()),
        Ok(output(
            ExecutionStatus::Exited { code: 1 },
            Vec::new(),
            Vec::new(),
        )),
    ]);

    let failure = backend
        .run(&one_shot(65_536))
        .expect_err("successful execution cannot hide failed cleanup");

    assert_eq!(
        failure,
        BackendError::CleanupFailed {
            instance_id: super::fixtures::instance(),
            primary_failed: false,
            cleanup: CommandFailure::new(
                Operation::Delete,
                CommandFailureReason::Status(ExecutionStatus::Exited { code: 1 }),
            ),
        }
    );
}

#[test]
fn one_shot_defensively_bounds_combined_output_from_any_runner() {
    let (backend, _) = backend([
        Ok(output(
            ExecutionStatus::Exited { code: 0 },
            b"abcdefgh".to_vec(),
            b"WXYZ1234".to_vec(),
        )),
        Ok(owned_inspection()),
        Ok(success(Vec::<u8>::new())),
    ]);

    let result = backend.run(&one_shot(10)).expect("cleanup succeeds");

    assert_eq!(result.status(), ExecutionStatus::OutputLimitExceeded);
    assert_eq!(result.stdout(), b"abcdefgh");
    assert_eq!(result.stderr(), b"WX");
    assert_eq!(result.stdout_observed_bytes(), 8);
    assert_eq!(result.stderr_observed_bytes(), 8);
}

#[test]
fn one_shot_preserves_non_utf8_guest_output_exactly() {
    let stdout = vec![0x00, 0xff, 0x80, b'A'];
    let stderr = vec![0xfe, b'B', 0x00];
    let (backend, _) = backend([
        Ok(output(
            ExecutionStatus::Exited { code: 0 },
            stdout.clone(),
            stderr.clone(),
        )),
        Ok(owned_inspection()),
        Ok(success(Vec::<u8>::new())),
    ]);

    let result = backend.run(&one_shot(65_536)).expect("cleanup succeeds");

    assert_eq!(result.stdout(), stdout);
    assert_eq!(result.stderr(), stderr);
    assert_eq!(result.stdout_observed_bytes(), 4);
    assert_eq!(result.stderr_observed_bytes(), 3);
    let encoded = serde_json::to_value(&result).expect("execution result is JSON safe");
    assert_eq!(encoded["stdout"]["encoding"], "base64");
    assert_eq!(encoded["stdout"]["byte_length"], 4);
    assert_eq!(encoded["stdout"]["data"], "AP+AQQ==");
    assert_eq!(encoded["stderr"]["encoding"], "base64");
    assert_eq!(encoded["stderr"]["byte_length"], 3);
    assert_eq!(encoded["stderr"]["data"], "/kIA");
}

#[test]
fn execution_result_rejects_combined_output_above_16_mib_before_encoding() {
    let result = crate::ExecutionResult::from_process(
        instance(),
        crate::process::ProcessOutput::new(
            ExecutionStatus::Exited { code: 0 },
            vec![b'x'; 8 * 1_048_576 + 1],
            vec![b'y'; 8 * 1_048_576],
            1,
        ),
        Some(crate::CleanupState::Complete),
        None,
        None,
    );

    let error = serde_json::to_vec(&result).expect_err("combined output must be bounded");

    assert!(error.to_string().contains("16 MiB"));
}

#[test]
fn request_and_error_debug_output_never_expose_guest_values() {
    let request = one_shot(65_536);
    let request_debug = format!("{request:?}");
    assert!(!request_debug.contains("node:22"));
    assert!(!request_debug.contains("/usr/local/bin/node"));
    assert!(!request_debug.contains("--version"));

    let (backend, _) = backend([
        Err(ProcessFailureKind::PermissionDenied),
        Ok(output(
            ExecutionStatus::Exited { code: 1 },
            Vec::new(),
            Vec::new(),
        )),
    ]);
    let failure = backend.run(&request).expect_err("cleanup cannot be proven");
    let error_json = serde_json::to_string(&failure).expect("error is JSON safe");
    assert!(!error_json.contains("node:22"));
    assert!(!error_json.contains("/usr/local/bin/node"));
    assert!(!error_json.contains("--version"));
}
