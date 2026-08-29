use crate::{BackendError, CleanupState, ExecuteCommand, ExecutionStatus, ProcessFailureKind};

use super::super::fixtures::{
    backend, execution_limits, instance, output, owned_inspection, success,
};

#[test]
fn terminally_uncertain_managed_exec_force_deletes_the_owned_vm_and_reports_cleanup() {
    for status in [
        ExecutionStatus::TimedOut,
        ExecutionStatus::OutputLimitExceeded,
        ExecutionStatus::Signaled,
    ] {
        let (backend, runner) = backend([
            Ok(owned_inspection()),
            Ok(output(status, Vec::new(), Vec::new())),
            Ok(owned_inspection()),
            Ok(success(Vec::<u8>::new())),
        ]);
        let request = ExecuteCommand::new(
            instance(),
            crate::GuestCommand::new("/bin/sleep", ["infinity"]).expect("command"),
            execution_limits(65_536),
        );

        let result = backend
            .execute(&request)
            .expect("terminal cleanup succeeds");

        assert_eq!(result.status(), status);
        assert_eq!(result.cleanup(), Some(CleanupState::Complete));
        assert_eq!(
            runner
                .calls()
                .iter()
                .map(|call| call.arguments[0].as_str())
                .collect::<Vec<_>>(),
            ["inspect", "exec", "inspect", "delete"]
        );
    }
}

#[test]
fn managed_exec_read_failure_force_deletes_the_owned_vm_and_exposes_invalidation() {
    let (backend, runner) = backend([
        Ok(owned_inspection()),
        Err(ProcessFailureKind::ReadFailed),
        Ok(owned_inspection()),
        Ok(success(Vec::<u8>::new())),
    ]);
    let request = ExecuteCommand::new(
        instance(),
        crate::GuestCommand::new("/bin/sleep", ["infinity"]).expect("command"),
        execution_limits(65_536),
    );

    let error = backend
        .execute(&request)
        .expect_err("read failure cannot prove guest termination");

    assert!(matches!(
        error,
        BackendError::ManagedExecutionInvalidated {
            cleanup: CleanupState::Complete,
            ..
        }
    ));
    assert_eq!(
        runner
            .calls()
            .iter()
            .map(|call| call.arguments[0].as_str())
            .collect::<Vec<_>>(),
        ["inspect", "exec", "inspect", "delete"]
    );
}
