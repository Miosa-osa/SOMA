use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt as _;

use soma_guest::{CommandContext, GuestCommand};

use super::user::{self, Credentials};
use super::{ENVIRONMENT, InvalidInvocation, Invocation, MAX_ARGUMENTS, MAX_FIELD_BYTES};

fn command(program: &[u8], arguments: &[&[u8]]) -> GuestCommand {
    GuestCommand::new(
        program.to_vec(),
        arguments.iter().map(|argument| argument.to_vec()).collect(),
        1_000,
        1,
    )
    .expect("bounded command")
}

fn with_context(context: CommandContext) -> GuestCommand {
    command(b"/bin/true", &[])
        .with_context(context)
        .expect("bounded context")
}

fn value(invocation: &Invocation, name: &str) -> Option<OsString> {
    invocation
        .environment()
        .iter()
        .find(|(existing, _)| existing == name)
        .map(|(_, value)| value.clone())
}

#[test]
fn converts_program_and_arguments_without_interpretation() {
    let invocation =
        Invocation::from_command(&command(b"/bin/echo", &[b"$HOME", b"a b", b"", b"\xff"]))
            .expect("valid invocation");

    assert_eq!(invocation.program(), "/bin/echo");
    assert_eq!(invocation.arguments().len(), 4);
    assert_eq!(invocation.arguments()[0], "$HOME");
    assert_eq!(invocation.arguments()[1], "a b");
    assert_eq!(invocation.arguments()[2], "");
    assert_eq!(invocation.arguments()[3].as_bytes(), b"\xff");
}

#[test]
fn the_base_environment_is_sorted_and_free_of_shells() {
    let names: Vec<&str> = ENVIRONMENT.iter().map(|(name, _)| *name).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();

    assert_eq!(names, sorted);
    assert!(ENVIRONMENT.iter().all(|(name, _)| name != &"SHELL"));
    assert_eq!(super::WORKING_DIRECTORY, "/");
}

#[test]
fn local_bounds_match_the_wire_contract() {
    assert_eq!(MAX_ARGUMENTS, 64);
    assert_eq!(MAX_FIELD_BYTES, 4096);
    assert_eq!(super::MAX_ENVIRONMENT, 64);
    let sixty_four: Vec<&[u8]> = vec![b"x"; 64];
    assert!(Invocation::from_command(&command(b"/bin/true", &sixty_four)).is_ok());
    assert!(GuestCommand::new(b"bin/true".to_vec(), vec![], 1, 1).is_err());
}

#[test]
fn a_command_without_a_context_runs_under_the_agents_own_policy() {
    let invocation =
        Invocation::from_command(&command(b"/bin/true", &[])).expect("valid invocation");

    assert_eq!(invocation.environment().len(), ENVIRONMENT.len());
    assert_eq!(invocation.working_directory(), "/");
    assert_eq!(invocation.credentials(), None);
}

#[test]
fn a_caller_pair_replaces_the_base_pair_of_the_same_name_and_keeps_the_rest() {
    let command = with_context(CommandContext {
        environment: vec![
            (b"PATH".to_vec(), b"/opt/bin".to_vec()),
            (b"TOKEN".to_vec(), b"s3cret".to_vec()),
            (b"EMPTY".to_vec(), Vec::new()),
        ],
        ..CommandContext::default()
    });

    let invocation = Invocation::from_command(&command).expect("valid invocation");

    assert_eq!(invocation.environment().len(), ENVIRONMENT.len() + 2);
    assert_eq!(value(&invocation, "PATH"), Some(OsString::from("/opt/bin")));
    assert_eq!(value(&invocation, "TOKEN"), Some(OsString::from("s3cret")));
    assert_eq!(value(&invocation, "EMPTY"), Some(OsString::new()));
    assert_eq!(value(&invocation, "HOME"), Some(OsString::from("/root")));
}

#[test]
fn a_named_working_directory_replaces_the_default() {
    let command = with_context(CommandContext {
        working_directory: Some(b"/work/space".to_vec()),
        ..CommandContext::default()
    });

    let invocation = Invocation::from_command(&command).expect("valid invocation");

    assert_eq!(invocation.working_directory(), "/work/space");
}

#[test]
fn an_unknown_user_refuses_the_invocation() {
    let command = with_context(CommandContext {
        user: Some(b"no-such-account-for-soma-tests".to_vec()),
        ..CommandContext::default()
    });

    assert_eq!(
        Invocation::from_command(&command).expect_err("unknown user"),
        InvalidInvocation::User
    );
}

#[test]
fn a_passwd_lookup_finds_the_named_account_and_skips_what_it_cannot_read() {
    let passwd = b"root:x:0:0:root:/root:/bin/sh\n\
        broken:x:notanumber:2:::\n\
        builder:x:1000:1001:Builder:/home/builder:/bin/sh\n\
        truncated:x\n";

    assert_eq!(
        user::lookup(passwd, b"builder"),
        Some(Credentials {
            uid: 1000,
            gid: 1001
        })
    );
    assert_eq!(
        user::lookup(passwd, b"root"),
        Some(Credentials { uid: 0, gid: 0 })
    );
    assert_eq!(user::lookup(passwd, b"broken"), None);
    assert_eq!(user::lookup(passwd, b"truncated"), None);
    assert_eq!(user::lookup(passwd, b"absent"), None);
}
