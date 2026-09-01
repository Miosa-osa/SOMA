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
