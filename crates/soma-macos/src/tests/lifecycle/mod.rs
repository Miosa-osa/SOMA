mod cleanup;
mod create;
mod execution;
mod inspect;
mod ownership;

use crate::{
    CleanupState, CreateMachine, ExecuteCommand, ExecutionStatus, ImageReference, MachineShape,
    Operation, StopOptions,
};

use super::fixtures::{
    INSTANCE, backend, control_limits, execution_limits, instance, node_command, owned_inspection,
    strings, success,
};

pub(super) fn create_request() -> CreateMachine {
    CreateMachine::new(
        instance(),
        ImageReference::new("node:22").expect("image"),
        MachineShape::new(1, 1_073_741_824).expect("shape"),
        crate::GuestCommand::new("/bin/sleep", ["infinity"]).expect("init command"),
        control_limits(),
    )
}

#[test]
fn managed_lifecycle_uses_the_verified_apple_command_contract() {
    let (backend, runner) = backend([
        Ok(success(Vec::<u8>::new())),
        Ok(owned_inspection()),
        Ok(success(Vec::<u8>::new())),
        Ok(owned_inspection()),
        Ok(success(b"v22.20.0\n".to_vec())),
        Ok(owned_inspection()),
        Ok(success(Vec::<u8>::new())),
        Ok(owned_inspection()),
        Ok(owned_inspection()),
        Ok(success(Vec::<u8>::new())),
    ]);
    let created = backend.create(&create_request()).expect("create succeeds");
    assert_eq!(created.container_name(), format!("soma-{INSTANCE}"));
    assert_eq!(
        backend
            .start(instance(), control_limits())
            .expect("start succeeds")
            .operation(),
        Operation::Start
    );
    let executed = backend
        .execute(&ExecuteCommand::new(
            instance(),
            node_command(),
            execution_limits(65_536),
        ))
        .expect("exec succeeds");
    assert_eq!(executed.status(), ExecutionStatus::Exited { code: 0 });
    assert_eq!(executed.cleanup(), None::<CleanupState>);
    backend
        .stop(instance(), StopOptions::new(2, control_limits()))
        .expect("stop succeeds");
    let inspected = backend
        .inspect(instance(), control_limits())
        .expect("inspect succeeds");
    assert_eq!(inspected.document()[0]["status"]["state"], "running");
    let resources = inspected.resources().expect("runtime resource evidence");
    assert_eq!(resources.vcpus(), 1);
    assert_eq!(resources.memory_bytes(), 1_073_741_824);
    backend
        .delete(instance(), control_limits())
        .expect("delete succeeds");

    let calls = runner.calls();
    assert_eq!(calls.len(), 10);
    assert_eq!(
        calls[0].arguments,
        strings(&[
            "create",
            "--name",
            &format!("soma-{INSTANCE}"),
            "--label",
            &format!("io.miosa.soma.instance={INSTANCE}"),
            "--cpus",
            "1",
            "--memory",
            "1024M",
            "--entrypoint",
            "/bin/sleep",
            "node:22",
            "infinity",
        ])
    );
    assert!(!calls[0].arguments.iter().any(|value| value == "--progress"));
    assert_eq!(
        calls[2].arguments,
        strings(&["start", &format!("soma-{INSTANCE}")])
    );
    assert_eq!(
        calls[4].arguments,
        strings(&[
            "exec",
            &format!("soma-{INSTANCE}"),
            "/usr/local/bin/node",
            "--version",
        ])
    );
    assert_eq!(
        calls[6].arguments,
        strings(&["stop", "--time", "2", &format!("soma-{INSTANCE}")])
    );
    assert_eq!(
        calls[7].arguments,
        strings(&["inspect", &format!("soma-{INSTANCE}")])
    );
    assert_eq!(
        calls[9].arguments,
        strings(&["delete", "--force", &format!("soma-{INSTANCE}")])
    );
    for index in [1, 3, 5, 7, 8] {
        assert_eq!(
            calls[index].arguments,
            strings(&["inspect", &format!("soma-{INSTANCE}")])
        );
    }
}
