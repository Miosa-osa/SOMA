//! The bounds this runner applies to a command, and the refusal a caller reads instead of an outage.

use soma_guest::{MAX_ARGUMENTS, MAX_FIELD_BYTES, MAX_TIMEOUT_MILLIS};

use super::{Refusal, admit};

fn word(bytes: usize) -> String {
    "a".repeat(bytes)
}

#[test]
fn a_command_at_every_bound_is_admitted() {
    assert_eq!(
        admit("/bin/sh", &["-lc", &word(MAX_FIELD_BYTES - 2)], 30_000),
        Ok(())
    );
    assert_eq!(
        admit(
            "/bin/true",
            vec!["x"; MAX_ARGUMENTS].as_slice(),
            u64::from(MAX_TIMEOUT_MILLIS)
        ),
        Ok(())
    );
}

#[test]
fn one_byte_over_a_field_bound_is_refused_by_name() {
    assert_eq!(
        admit("/bin/sh", &[&word(MAX_FIELD_BYTES + 1)], 30_000),
        Err(Refusal::TooLarge {
            field: "argument",
            limit: MAX_FIELD_BYTES,
            actual: MAX_FIELD_BYTES + 1,
        })
    );
}

#[test]
fn one_argument_too_many_is_refused_by_name() {
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
fn fields_that_fit_alone_but_not_together_are_refused_as_a_whole() {
    let argument = word(1024);
    let arguments = vec![argument.as_str(); 64];
    let refusal = admit("/bin/true", &arguments, 30_000).expect_err("too large together");
    let Refusal::TooLargeTogether { limit, actual } = refusal else {
        panic!("{refusal:?}");
    };
    assert!(
        actual > limit,
        "{actual} must be past the {limit} a message carries"
    );
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
            vec![word(MAX_FIELD_BYTES)],
            u64::from(MAX_TIMEOUT_MILLIS),
        ),
        (
            "/bin/true".to_owned(),
            vec![word(MAX_FIELD_BYTES); MAX_ARGUMENTS],
            30_000,
        ),
        ("/bin/true".to_owned(), vec![word(4096); 16], 30_000),
    ];
    for (program, arguments, timeout) in shapes {
        let borrowed: Vec<&str> = arguments.iter().map(String::as_str).collect();
        if admit(&program, &borrowed, timeout).is_err() {
            continue;
        }
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
}

/// Every refusal is an answer a caller can act on, and none of them is an outage.
#[test]
fn every_refusal_answers_a_specific_code_rather_than_an_outage() {
    let cases = [
        (
            Refusal::TooLarge {
                field: "argument",
                limit: MAX_FIELD_BYTES,
                actual: MAX_FIELD_BYTES + 1,
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
fn the_facade_type_and_the_machine_state_one_exec_contract() {
    // The runner's own request type used to admit far more than any machine could run, which is
    // how a command reached the engine, was refused there, and took the sandbox with it. The two
    // sets are now the same set, and this is what says so: change one bound alone and this fails.
    use soma::DirectCommand;
    use soma_guest::{FIXED_BODY_SIZE, MAX_BODY_SIZE};

    assert_eq!(DirectCommand::MAX_EXECUTABLE_BYTES, MAX_FIELD_BYTES);
    assert_eq!(DirectCommand::MAX_ARGUMENTS, MAX_ARGUMENTS);
    assert_eq!(DirectCommand::MAX_ARGUMENT_BYTES, MAX_FIELD_BYTES);
    assert_eq!(
        DirectCommand::MAX_AGGREGATE_BYTES,
        MAX_BODY_SIZE - FIXED_BODY_SIZE
    );
}

#[test]
fn every_command_the_shared_type_admits_is_one_the_machine_can_be_asked_to_run() {
    use soma::DirectCommand;

    // The aggregate is the binding bound here rather than the count: fifteen full fields and the
    // remainder spend exactly one guest record's body, and both layers take exactly that.
    let full =
        DirectCommand::MAX_AGGREGATE_BYTES - ("/bin/true".len() + 15 * (2 + MAX_FIELD_BYTES));
    let arguments = |last: usize| {
        let mut arguments = vec!["a".repeat(MAX_FIELD_BYTES); 15];
        arguments.push("a".repeat(last));
        arguments
    };
    let admitted =
        DirectCommand::new("/bin/true", arguments(full - 2)).expect("the machine carries it");
    assert_eq!(
        admit(
            admitted.executable(),
            &admitted
                .arguments()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            30_000,
        ),
        Ok(())
    );

    // One byte more is past both: the shared type refuses it, and so does the machine's own
    // contract, which is the property that keeps a command out of the engine entirely.
    let over = arguments(full - 1);
    assert!(DirectCommand::new("/bin/true", over.clone()).is_err());
    assert!(
        admit(
            "/bin/true",
            &over.iter().map(String::as_str).collect::<Vec<_>>(),
            30_000
        )
        .is_err()
    );

    // And the two countable bounds agree too.
    assert!(DirectCommand::new("/bin/true", vec!["a".repeat(MAX_FIELD_BYTES + 1)]).is_err());
    assert!(DirectCommand::new("/bin/true", vec!["x"; MAX_ARGUMENTS + 1]).is_err());
}
