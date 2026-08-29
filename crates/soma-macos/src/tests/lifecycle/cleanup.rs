use crate::{BackendError, CreateMachine, ExecutionStatus, ImageReference, MachineShape};

use super::{
    super::fixtures::{
        INSTANCE, backend, control_limits, execution_limits, instance, output, owned_inspection,
        success,
    },
    create_request,
};

#[test]
fn failed_create_attempts_force_cleanup() {
    let (backend, runner) = backend([
        Ok(output(
            ExecutionStatus::Exited { code: 125 },
            Vec::new(),
            Vec::new(),
        )),
        Ok(owned_inspection()),
        Ok(success(Vec::<u8>::new())),
    ]);
    let create = CreateMachine::new(
        instance(),
        ImageReference::new("node:22").expect("image"),
        MachineShape::new(1, 1_073_741_824).expect("shape"),
        crate::GuestCommand::new("/bin/sleep", ["infinity"]).expect("init command"),
        control_limits(),
    );

    backend
        .create(&create)
        .expect_err("failed create is not a Machine");

    assert_eq!(runner.calls().len(), 3);
    assert_eq!(runner.calls()[1].arguments[0], "inspect");
    assert_eq!(runner.calls()[2].arguments[0], "delete");
}

#[test]
fn create_collision_never_deletes_a_container_without_exact_soma_ownership() {
    let unowned = format!(
        r#"[{{"configuration":{{"id":"soma-{INSTANCE}","labels":{{"owner":"someone-else"}}}},"id":"soma-{INSTANCE}"}}]"#
    );
    let (backend, runner) = backend([
        Ok(output(
            ExecutionStatus::Exited { code: 125 },
            Vec::new(),
            Vec::new(),
        )),
        Ok(success(unowned.into_bytes())),
    ]);

    backend
        .create(&create_request())
        .expect_err("name collision is not owned cleanup authority");

    let calls = runner.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].arguments[0], "create");
    assert_eq!(calls[1].arguments[0], "inspect");
    assert!(!calls.iter().any(|call| call.arguments[0] == "delete"));
}

#[test]
fn terminal_managed_exec_returns_cleanup_uncertainty_when_delete_fails() {
    let (backend, _) = backend([
        Ok(owned_inspection()),
        Ok(output(ExecutionStatus::TimedOut, Vec::new(), Vec::new())),
        Ok(owned_inspection()),
        Ok(output(
            ExecutionStatus::Exited { code: 1 },
            Vec::new(),
            Vec::new(),
        )),
    ]);
    let request = crate::ExecuteCommand::new(
        instance(),
        crate::GuestCommand::new("/bin/sleep", ["infinity"]).expect("command"),
        execution_limits(65_536),
    );

    let error = backend
        .execute(&request)
        .expect_err("terminal execution without deletion proof is uncertain");

    assert!(matches!(
        error,
        BackendError::CleanupFailed {
            primary_failed: true,
            ..
        }
    ));
}
