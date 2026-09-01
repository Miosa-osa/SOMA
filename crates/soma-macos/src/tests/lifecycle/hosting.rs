//! A machine this adapter creates is held by the Apple runtime service, not by this process.
//!
//! Nothing in [`MacOsBackend`](crate::MacOsBackend) is a handle to a machine: it is an
//! executable path and a process runner. Every operation spawns one `container` process that
//! names the machine `soma-<instance>` and re-proves ownership from the service's own record.
//! So the only thing a later process needs in order to drive a machine is the Instance identity,
//! which is what makes a launched identity usable after the launching command has exited.
//!
//! The test below is the mechanism, not the outcome: each stage runs on a backend value built
//! from scratch, with its own runner, that never saw the one before it.

use crate::{ExecuteCommand, ExecutionStatus, StopOptions};

use super::{
    super::fixtures::{
        INSTANCE, backend, control_limits, execution_limits, instance, node_command,
        owned_inspection, strings, success,
    },
    create_request,
};

#[test]
fn a_machine_is_driven_from_processes_that_did_not_create_it() {
    // The launching process: it creates and starts the machine, then goes away.
    let (launcher, launcher_calls) = backend([
        Ok(success(Vec::<u8>::new())),
        Ok(owned_inspection()),
        Ok(success(Vec::<u8>::new())),
    ]);
    launcher.create(&create_request()).expect("create");
    launcher.start(instance(), control_limits()).expect("start");
    let created = launcher_calls.calls();
    drop(launcher);

    // A second process, holding only the Instance identity, runs a command in that machine.
    let (commander, commander_calls) =
        backend([Ok(owned_inspection()), Ok(success(b"v22.20.0\n".to_vec()))]);
    let executed = commander
        .execute(&ExecuteCommand::new(
            instance(),
            node_command(),
            execution_limits(65_536),
        ))
        .expect("a machine this backend never created answers a command");
    assert_eq!(executed.status(), ExecutionStatus::Exited { code: 0 });
    drop(commander);

    // A third process releases it, again knowing nothing but the identity.
    let (releaser, releaser_calls) = backend([
        Ok(owned_inspection()),
        Ok(success(Vec::<u8>::new())),
        Ok(owned_inspection()),
        Ok(success(Vec::<u8>::new())),
    ]);
    releaser
        .stop(instance(), StopOptions::new(2, control_limits()))
        .expect("stop");
    releaser
        .delete(instance(), control_limits())
        .expect("delete");

    // Every process addressed the same machine by the same name, and that name is a function of
    // the Instance identity alone: nothing was carried from one process to the next.
    let name = format!("soma-{INSTANCE}");
    assert_eq!(
        created[0].arguments,
        strings(&[
            "create",
            "--name",
            &name,
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
    assert_eq!(
        commander_calls.calls()[1].arguments,
        strings(&["exec", &name, "/usr/local/bin/node", "--version"])
    );
    assert_eq!(
        releaser_calls.calls()[3].arguments,
        strings(&["delete", "--force", &name])
    );
    // Ownership is re-proved from the service's record before each operation rather than
    // remembered, which is what lets an unrelated process act on the machine at all.
    for calls in [&commander_calls.calls()[0], &releaser_calls.calls()[0]] {
        assert_eq!(calls.arguments, strings(&["inspect", &name]));
    }
}
