//! The bounds this runner applies to a command, and the refusal a caller reads instead of an outage.
//!
//! Every bound is checked one byte (or one element) under, at, and over, because the property a
//! caller relies on is exact: a command at a bound runs and the next one up is refused by name.

use soma_guest::{
    FIXED_BODY_SIZE, MAX_ARGUMENT_BYTES, MAX_ARGUMENTS, MAX_BODY_SIZE, MAX_FIELD_BYTES,
    MAX_TIMEOUT_MILLIS,
};

use super::super::command::exec_command;
use super::{Refusal, admit};

fn word(bytes: usize) -> String {
    "a".repeat(bytes)
}

#[test]
fn one_argument_is_bounded_exactly() {
    for bytes in [MAX_ARGUMENT_BYTES - 1, MAX_ARGUMENT_BYTES] {
        assert_eq!(
            admit("/bin/sh", &["-lc", &word(bytes)], 30_000),
            Ok(()),
            "{bytes}"
        );
    }
    assert_eq!(
        admit("/bin/sh", &["-lc", &word(MAX_ARGUMENT_BYTES + 1)], 30_000),
        Err(Refusal::TooLarge {
            field: "argument",
            limit: MAX_ARGUMENT_BYTES,
            actual: MAX_ARGUMENT_BYTES + 1,
        })
    );
}

#[test]
fn the_argument_count_is_bounded_exactly() {
    for count in [MAX_ARGUMENTS - 1, MAX_ARGUMENTS] {
        assert_eq!(
            admit("/bin/true", vec!["x"; count].as_slice(), 30_000),
            Ok(()),
            "{count}"
        );
    }
    assert_eq!(
        admit("/bin/true", vec!["x"; MAX_ARGUMENTS + 1].as_slice(), 30_000),
        Err(Refusal::TooLarge {
            field: "argument count",
            limit: MAX_ARGUMENTS,
            actual: MAX_ARGUMENTS + 1,
        })
    );
}

#[test]
fn the_program_is_bounded_exactly() {
    let program = |bytes: usize| format!("/{}", word(bytes - 1));
    for bytes in [MAX_FIELD_BYTES - 1, MAX_FIELD_BYTES] {
        assert_eq!(admit(&program(bytes), &[], 30_000), Ok(()), "{bytes}");
    }
    assert_eq!(
        admit(&program(MAX_FIELD_BYTES + 1), &[], 30_000),
        Err(Refusal::TooLarge {
            field: "program",
            limit: MAX_FIELD_BYTES,
            actual: MAX_FIELD_BYTES + 1,
        })
    );
}

/// One full argument and a second of `last` bytes: no single argument can fill a record alone.
fn filling(last: usize) -> Vec<String> {
    vec![word(MAX_ARGUMENT_BYTES), word(last)]
}

/// The second argument's length that puts a `/bin/true` command exactly at `bytes` of body.
fn last_for(bytes: usize) -> usize {
    bytes - (FIXED_BODY_SIZE + "/bin/true".len() + (2 + MAX_ARGUMENT_BYTES) + 2)
}

#[test]
fn the_whole_command_is_bounded_by_one_record_exactly_and_the_refusal_names_its_size() {
    for bytes in [MAX_BODY_SIZE - 1, MAX_BODY_SIZE] {
        let arguments = filling(last_for(bytes));
        let borrowed: Vec<&str> = arguments.iter().map(String::as_str).collect();
        assert_eq!(admit("/bin/true", &borrowed, 30_000), Ok(()), "{bytes}");
    }
    let arguments = filling(last_for(MAX_BODY_SIZE + 1));
    let borrowed: Vec<&str> = arguments.iter().map(String::as_str).collect();
    assert_eq!(
        admit("/bin/true", &borrowed, 30_000),
        Err(Refusal::TooLargeTogether {
            limit: MAX_BODY_SIZE,
            actual: MAX_BODY_SIZE + 1,
        })
    );
}

#[test]
fn a_twelve_kilobyte_heredoc_is_admitted_and_reaches_the_guest_as_one_argument() {
    // ComputeSDK's cpu-burst command is two heredocs and a node run in one string, about 12 KB.
    let heredoc = format!(
        "cat > /tmp/w.js <<'EOF'\n{}\nEOF\nnode /tmp/w.js",
        "// x\n".repeat(2_400)
    );
    assert!(heredoc.len() > 12_000);
    let command = exec_command(&heredoc, false, 30_000).expect("admitted");
    assert_eq!(command.executable(), "/bin/sh");
    assert_eq!(command.arguments(), ["-lc".to_owned(), heredoc]);
}

#[test]
fn a_deadline_past_the_runtime_bound_is_refused_rather_than_reaching_the_guest() {
    assert_eq!(
        admit("/bin/true", &[], u64::from(MAX_TIMEOUT_MILLIS) + 1),
        Err(Refusal::TimeoutUnsupported {
            limit_millis: MAX_TIMEOUT_MILLIS,
            actual_millis: u64::from(MAX_TIMEOUT_MILLIS) + 1,
        })
    );
}

/// The property the fix rests on: nothing admitted here is a command the guest protocol refuses.
///
/// The two bounds were free to disagree, which is how an oversize command reached the engine. This
/// pins them together over the shapes that sit on either side of every bound.
#[test]
fn everything_admitted_here_is_a_command_the_guest_protocol_carries() {
    let shapes: Vec<(String, Vec<String>, u64)> = vec![
        (
            "/bin/sh".to_owned(),
            vec!["-lc".to_owned(), "echo hi".to_owned()],
            30_000,
        ),
        ("/bin/true".to_owned(), vec![], 1),
        (
            "/bin/true".to_owned(),
            vec![word(MAX_ARGUMENT_BYTES)],
            u64::from(MAX_TIMEOUT_MILLIS),
        ),
        (
            "/bin/true".to_owned(),
            vec![word(MAX_FIELD_BYTES); MAX_ARGUMENTS],
            30_000,
        ),
        (
            "/bin/true".to_owned(),
            filling(last_for(MAX_BODY_SIZE)),
            30_000,
        ),
        (
            "/bin/true".to_owned(),
            filling(last_for(MAX_BODY_SIZE + 1)),
            30_000,
        ),
        ("/bin/true".to_owned(), vec![word(4096); 16], 30_000),
    ];
    let mut admitted = 0;
    for (program, arguments, timeout) in shapes {
        let borrowed: Vec<&str> = arguments.iter().map(String::as_str).collect();
        if admit(&program, &borrowed, timeout).is_err() {
            continue;
        }
        admitted += 1;
        let timeout_millis = u32::try_from(timeout).expect("an admitted deadline");
        assert!(
            soma_guest::GuestCommand::new(
                program.clone().into_bytes(),
                arguments
                    .iter()
                    .map(|argument| argument.as_bytes().to_vec())
                    .collect(),
                timeout_millis,
                soma::ExecutionLimits::DEFAULT_MAX_OUTPUT_BYTES,
            )
            .is_ok(),
            "admitted but not carried: {program} with {} arguments",
            arguments.len()
        );
    }
    assert_eq!(admitted, 4, "the shapes inside every bound are admitted");
}

/// Every refusal is an answer a caller can act on, and none of them is an outage.
#[test]
fn every_refusal_answers_a_specific_code_rather_than_an_outage() {
    let cases = [
        (
            Refusal::TooLarge {
                field: "argument",
                limit: MAX_ARGUMENT_BYTES,
                actual: MAX_ARGUMENT_BYTES + 1,
            },
            "RUNNER_COMMAND_TOO_LARGE",
            413,
        ),
        (
            Refusal::TooLargeTogether {
                limit: 100,
                actual: 200,
            },
            "RUNNER_COMMAND_TOO_LARGE",
            413,
        ),
        (
            Refusal::TimeoutUnsupported {
                limit_millis: MAX_TIMEOUT_MILLIS,
                actual_millis: u64::from(MAX_TIMEOUT_MILLIS) + 1,
            },
            "EXEC_TIMEOUT_UNSUPPORTED",
            400,
        ),
        (Refusal::Invalid, "INVALID_PARAM", 400),
    ];
    for (refusal, code, status) in cases {
        let error = refusal.error();
        assert_eq!((error.code, error.status), (code, status), "{refusal:?}");
        assert!(!error.retryable, "{refusal:?} must not invite a retry");
        assert!(
            error.details.is_some(),
            "{refusal:?} must name what was wrong"
        );
    }
}

#[test]
fn a_size_refusal_names_the_limit_and_the_size_in_its_message_and_details() {
    let error = Refusal::TooLarge {
        field: "argument",
        limit: MAX_ARGUMENT_BYTES,
        actual: MAX_ARGUMENT_BYTES + 1,
    }
    .error();
    assert!(
        error.message.contains(&format!(
            "argument is {} bytes, the limit is {MAX_ARGUMENT_BYTES} bytes",
            MAX_ARGUMENT_BYTES + 1
        )),
        "{}",
        error.message
    );
    let details = error.details.expect("details");
    assert_eq!(details["field"], "argument");
    assert_eq!(details["limit"], MAX_ARGUMENT_BYTES);
    assert_eq!(details["actual"], MAX_ARGUMENT_BYTES + 1);

    let count = Refusal::TooLarge {
        field: "argument count",
        limit: MAX_ARGUMENTS,
        actual: MAX_ARGUMENTS + 1,
    }
    .error();
    assert!(
        count.message.contains(&format!(
            "argument count is {}, the limit is {MAX_ARGUMENTS}",
            MAX_ARGUMENTS + 1
        )),
        "{}",
        count.message
    );
}
