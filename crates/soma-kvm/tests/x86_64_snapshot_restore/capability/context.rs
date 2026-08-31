//! Live proof that a command runs with the environment, directory, account, and input it was
//! given.
//!
//! Each of the four is asserted by the process itself rather than by the protocol. The command
//! prints what its own environment holds, what its own working directory is, and what account
//! the kernel says it is running as, and it copies its standard input to its output. Nothing in
//! that answer can be produced by a codec that carried the fields and an agent that dropped
//! them.
//!
//! The base environment is asserted at the same time. A caller that names one variable must not
//! lose the `PATH` the agent supplies, because a command that cannot find its own tools is not
//! a command that ran with the caller's environment.

use soma_guest::{CommandContext, GuestCommand};
use soma_kvm::x86_64::SandboxMachine;

use crate::{
    x86_64_sandbox_boot_host::require_kvm,
    x86_64_sandbox_boot_session as session,
    x86_64_snapshot_restore_capability::{WORKSPACE, assert_no_leak, shell, succeeded},
    x86_64_snapshot_restore_fixture as fixture, x86_64_snapshot_restore_instance as instance,
    x86_64_snapshot_restore_workload::{self as workload, Session, Workload},
};

/// The account the command is asked to run as, which every Debian-derived image carries.
const ACCOUNT: &[u8] = b"nobody";
/// The account's identifier in that image, which the command must report as its own.
const ACCOUNT_UID: &str = "65534";
/// The variable the caller names, and the value only this run uses.
const NAMED: &[u8] = b"SOMA_LIVE_CONTEXT";
const NAMED_VALUE: &[u8] = b"a value only this run carries";
/// The bytes handed to the command's standard input.
const INPUT: &[u8] = b"one line the host typed\nand a second\n";
/// The script that reports all four, in fields the test can split.
const REPORT: &[u8] = b"printf '%s\\n%s\\n%s\\n%s\\n--\\n' \
     \"$SOMA_LIVE_CONTEXT\" \"$(pwd)\" \"$(id -u)\" \"$PATH\"; cat";
/// The directory the command is told to run in, made by the step before it.
const MAKE: &[u8] = b"mkdir -p /tmp/soma-live && chmod 0755 /tmp/soma-live && echo made";

/// What the context proof retains.
pub struct Context {
    pub made: String,
    pub reported: session::Executed,
}

struct ContextWorkload;

impl Workload for ContextWorkload {
    type Output = Context;

    fn run<'a>(
        &mut self,
        _machine: &'a SandboxMachine,
        session: Session<'a>,
    ) -> Result<(Session<'a>, Context), String> {
        let (session, executed) = workload::execute(session, &shell(&[b"-c", MAKE]))?;
        let made = succeeded("make", &executed);
        let command = GuestCommand::new(
            b"/bin/sh".to_vec(),
            vec![b"-c".to_vec(), REPORT.to_vec()],
            60_000,
            262_144,
        )
        .map_err(|error| format!("command: {error}"))?
        .with_context(CommandContext {
            environment: vec![(NAMED.to_vec(), NAMED_VALUE.to_vec())],
            working_directory: Some(WORKSPACE.to_vec()),
            user: Some(ACCOUNT.to_vec()),
            stdin: INPUT.to_vec(),
        })
        .map_err(|error| format!("context: {error}"))?;
        let (session, reported) = workload::run_command(session, command)?;
        Ok((session, Context { made, reported }))
    }
}

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
fn a_command_sees_the_environment_directory_user_and_standard_input_it_was_given() {
    require_kvm();
    let fixture = fixture::shared();
    let restored = instance::run_workload(&fixture, "context", 44, ContextWorkload);
    assert_no_leak(&restored);

    assert_eq!(restored.output.made.trim(), "made");
    let printed = succeeded("context", &restored.output.reported);
    eprintln!("[context] the command reported:\n{printed}");
    let (header, echoed) = printed
        .split_once("--\n")
        .unwrap_or_else(|| panic!("the command printed no field separator: {printed:?}"));
    let fields: Vec<&str> = header.lines().collect();
    assert_eq!(fields.len(), 4, "fields={fields:?}");

    assert_eq!(
        fields[0],
        String::from_utf8_lossy(NAMED_VALUE),
        "the command did not see the variable the caller named"
    );
    assert_eq!(
        fields[1],
        String::from_utf8_lossy(WORKSPACE),
        "the command did not run in the directory it was given"
    );
    assert_eq!(
        fields[2], ACCOUNT_UID,
        "the command did not run as the account it was given"
    );
    // The caller named one variable and said nothing about the rest, so the agent's own base
    // must still be there; a command with no PATH could not have found `id` at all.
    assert!(
        fields[3].contains("/usr/local/bin"),
        "the agent's base environment did not survive the caller's own: {:?}",
        fields[3]
    );
    assert_eq!(
        echoed,
        String::from_utf8_lossy(INPUT),
        "the command did not read the standard input it was handed"
    );
}
