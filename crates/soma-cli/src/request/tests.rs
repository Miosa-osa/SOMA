use clap::Parser as _;

use super::{PreparedOperation, RequestError, prepare_machine, prepare_run};
use crate::cli::{Cli, RootCommand};

#[test]
fn prepares_facade_run_with_generated_canonical_identities() {
    let cli = Cli::try_parse_from([
        "soma",
        "run",
        "--network",
        "denied",
        "node:22",
        "--",
        "/usr/local/bin/node",
        "--version",
    ])
    .expect("run syntax");
    let RootCommand::Run(arguments) = cli.command else {
        panic!("run command");
    };
    let PreparedOperation::Run { instance_id, .. } =
        prepare_run(arguments).expect("facade request")
    else {
        panic!("run request");
    };
    assert_eq!(instance_id.as_str().len(), 32);
}

#[test]
fn rejects_relative_guest_executable_before_runtime_work() {
    let cli = Cli::try_parse_from(["soma", "run", "node:22", "--", "node"])
        .expect("parser accepts bounded strings");
    let RootCommand::Run(arguments) = cli.command else {
        panic!("run command");
    };
    assert_eq!(prepare_run(arguments).err(), Some(RequestError::Command));
}

#[test]
fn prepares_managed_control_with_explicit_operation_identity() {
    let cli = Cli::try_parse_from([
        "soma",
        "machine",
        "destroy",
        "--operation-id",
        "11111111111111111111111111111111",
        "--instance-id",
        "22222222222222222222222222222222",
    ])
    .expect("destroy syntax");
    let RootCommand::Machine(arguments) = cli.command else {
        panic!("machine command");
    };
    let PreparedOperation::Destroy { instance_id, .. } =
        prepare_machine(arguments).expect("destroy request")
    else {
        panic!("destroy request");
    };
    assert_eq!(instance_id.as_str(), "22222222222222222222222222222222");
}

const RESIZE_INSTANCE: &str = "0123456789abcdef0123456789abcdef";

fn resize(extra: &[&str]) -> Result<PreparedOperation, RequestError> {
    let mut arguments = vec![
        "soma",
        "machine",
        "resize",
        "--instance-id",
        RESIZE_INSTANCE,
    ];
    arguments.extend_from_slice(extra);
    let cli = Cli::try_parse_from(arguments).expect("resize syntax");
    let RootCommand::Machine(machine) = cli.command else {
        panic!("machine command");
    };
    prepare_machine(machine)
}

#[test]
fn a_resize_carries_exactly_the_dimensions_that_were_asked_for() {
    let PreparedOperation::Resize { request, .. } =
        resize(&["--cpu", "2", "--memory-mib", "4096"]).expect("both")
    else {
        panic!("resize request");
    };
    assert_eq!(
        (request.cpu_count, request.memory_mb),
        (Some(2), Some(4096))
    );
    let PreparedOperation::Resize { request, .. } =
        resize(&["--memory-mib", "3072"]).expect("memory")
    else {
        panic!("resize request");
    };
    assert_eq!((request.cpu_count, request.memory_mb), (None, Some(3072)));
}

#[test]
fn a_resize_that_asks_for_nothing_is_refused_before_the_runtime_is_touched() {
    assert_eq!(resize(&[]).err(), Some(RequestError::Shape));
}

#[test]
fn a_resize_names_its_sandbox_by_the_canonical_identity_or_not_at_all() {
    let cli = Cli::try_parse_from([
        "soma",
        "machine",
        "resize",
        "--instance-id",
        "x",
        "--cpu",
        "1",
    ])
    .expect("syntax");
    let RootCommand::Machine(machine) = cli.command else {
        panic!("machine command");
    };
    assert_eq!(prepare_machine(machine).err(), Some(RequestError::Identity));
    assert!(Cli::try_parse_from(["soma", "machine", "resize", "--cpu", "1"]).is_err());
}
