//! What a command's own environment, working directory, user, and standard input do to the
//! process the executor spawns.

use soma_guest::{CommandContext, GuestCommand, TerminalStatus};

use crate::environment::InvalidInvocation;

use super::super::{ExecutorFault, execute};
use super::{RecordingSink, command, run};

fn with_context(program: &str, arguments: &[&str], context: CommandContext) -> GuestCommand {
    command(program, arguments, 5_000, 4096)
        .with_context(context)
        .expect("bounded context")
}

#[test]
fn the_environment_is_the_base_allowlist_when_the_command_names_none() {
    let (completion, sink) = run(&command("/usr/bin/env", &[], 5_000, 4096));

    assert_eq!(completion.status, TerminalStatus::Exited(0));
    let text = String::from_utf8_lossy(&sink.stdout);
    assert!(text.contains("SOMA_SANDBOX=1\n"));
    assert!(text.contains("HOME=/root\n"));
    assert!(!text.contains("USER="));
    assert!(!text.contains("CARGO"));
}

#[test]
fn a_named_pair_replaces_the_base_one_and_a_new_name_is_added() {
    let command = with_context(
        "/usr/bin/env",
        &[],
        CommandContext {
            environment: vec![
                (b"PATH".to_vec(), b"/opt/bin".to_vec()),
                (b"TOKEN".to_vec(), b"from-the-caller".to_vec()),
            ],
            ..CommandContext::default()
        },
    );

    let (completion, sink) = run(&command);

    assert_eq!(completion.status, TerminalStatus::Exited(0));
    let text = String::from_utf8_lossy(&sink.stdout);
    assert!(text.contains("PATH=/opt/bin\n"), "{text}");
    assert!(text.contains("TOKEN=from-the-caller\n"), "{text}");
    assert!(text.contains("HOME=/root\n"), "{text}");
    // The base pair was replaced rather than joined by a second one of the same name.
    assert_eq!(text.matches("PATH=").count(), 1, "{text}");
}

#[test]
fn the_child_runs_in_the_working_directory_the_command_names() {
    let command = with_context(
        "/bin/pwd",
        &[],
        CommandContext {
            working_directory: Some(b"/tmp".to_vec()),
            ..CommandContext::default()
        },
    );

    let (completion, sink) = run(&command);

    assert_eq!(completion.status, TerminalStatus::Exited(0));
    assert_eq!(sink.stdout, b"/tmp\n");
}

#[test]
fn standard_input_reaches_the_child_and_then_ends() {
    let input = b"first line\nsecond line\n";
    let command = with_context(
        "/bin/cat",
        &[],
        CommandContext {
            stdin: input.to_vec(),
            ..CommandContext::default()
        },
    );

    let (completion, sink) = run(&command);

    // The child read every byte and then saw the end of its input, so it exited on its own
    // rather than being killed at the deadline.
    assert_eq!(completion.status, TerminalStatus::Exited(0));
    assert_eq!(sink.stdout, input);
}

#[test]
fn a_command_without_standard_input_reads_the_end_immediately() {
    let (completion, sink) = run(&command("/bin/cat", &[], 5_000, 4096));

    assert_eq!(completion.status, TerminalStatus::Exited(0));
    assert!(sink.stdout.is_empty());
}

#[test]
fn a_user_the_guest_does_not_know_is_refused_before_anything_is_spawned() {
    let command = with_context(
        "/bin/true",
        &[],
        CommandContext {
            user: Some(b"no-such-account-for-soma-tests".to_vec()),
            ..CommandContext::default()
        },
    );
    let mut sink = RecordingSink::default();

    let fault = execute(&command, &mut sink).expect_err("an unknown user");

    assert_eq!(fault, ExecutorFault::Invocation(InvalidInvocation::User));
    assert!(sink.stdout.is_empty() && sink.stderr.is_empty());
}
