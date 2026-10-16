//! A sandbox's persisted environment reaches a direct command, below what the command names and
//! above the agent's own base, and a change is seen by the next command with no restart.

use std::fs;
use std::os::unix::fs::PermissionsExt as _;

use soma_guest::{CommandContext, TerminalStatus};

use crate::environment::persisted::test_support::point_at;

use super::{command, run};

fn write_env(path: &std::path::Path, text: &str, mode: u32) {
    fs::write(path, text).expect("write");
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("mode");
}

fn env_of(program_command: &soma_guest::GuestCommand) -> String {
    let (completion, sink) = run(program_command);
    assert_eq!(completion.status, TerminalStatus::Exited(0));
    String::from_utf8_lossy(&sink.stdout).into_owned()
}

#[test]
fn a_direct_argv_command_sees_the_persisted_value_and_its_update_without_a_restart() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("identity.env");
    let _pointed = point_at(&path);
    let plain = command("/usr/bin/env", &[], 5_000, 4096);

    write_env(&path, "SOMA_PERSISTED='one'\nOTHER='x'\n", 0o600);
    let first = env_of(&plain);
    assert!(first.contains("SOMA_PERSISTED=one\n"), "{first}");
    assert!(first.contains("OTHER=x\n"), "{first}");
    assert!(
        first.contains("SOMA_SANDBOX=1\n"),
        "the base is still there: {first}"
    );

    write_env(&path, "SOMA_PERSISTED='two'\n", 0o600);
    let second = env_of(&plain);
    assert!(second.contains("SOMA_PERSISTED=two\n"), "{second}");
    assert!(
        !second.contains("OTHER="),
        "a removed variable is gone: {second}"
    );

    fs::remove_file(&path).expect("remove");
    assert!(!env_of(&plain).contains("SOMA_PERSISTED"));
}

#[test]
fn the_commands_own_pair_beats_the_persisted_one_which_beats_the_base() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("identity.env");
    let _pointed = point_at(&path);
    write_env(
        &path,
        "PATH='/persisted/bin'\nTOKEN='persisted'\nHOME='/persisted/home'\n",
        0o600,
    );

    let named = command("/usr/bin/env", &[], 5_000, 4096)
        .with_context(CommandContext {
            environment: vec![(b"TOKEN".to_vec(), b"command".to_vec())],
            ..CommandContext::default()
        })
        .expect("context");
    let text = env_of(&named);
    assert!(text.contains("TOKEN=command\n"), "{text}");
    assert!(
        text.contains("HOME=/persisted/home\n"),
        "persisted beats base: {text}"
    );
    assert!(text.contains("PATH=/persisted/bin\n"), "{text}");
    assert_eq!(text.matches("TOKEN=").count(), 1, "{text}");
}

#[test]
fn a_file_anyone_could_have_written_is_not_believed() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("identity.env");
    let _pointed = point_at(&path);
    write_env(&path, "SOMA_PERSISTED='planted'\n", 0o666);
    let text = env_of(&command("/usr/bin/env", &[], 5_000, 4096));
    assert!(!text.contains("SOMA_PERSISTED"), "{text}");
}
