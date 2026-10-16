//! The facade's own bounds and the guest protocol's, pinned to each other.
//!
//! The runner's request types and the machine's protocol are two doors into the same engine, and
//! these tests are what keeps the two sets of bounds one set.

use soma_guest::{MAX_ARGUMENTS, MAX_FIELD_BYTES};

use super::exec_contract::{Refusal, admit, admit_in};

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

#[test]
fn the_facade_context_bounds_are_the_guests() {
    use soma::{
        CommandContext, DirectCommand, MAX_CONTEXT_FIELD_BYTES, MAX_ENVIRONMENT, MAX_STDIN_BYTES,
        MAX_USER_BYTES,
    };
    use soma_guest::{FIXED_BODY_SIZE, MAX_BODY_SIZE};

    assert_eq!(MAX_ENVIRONMENT, soma_guest::MAX_ENVIRONMENT);
    assert_eq!(MAX_CONTEXT_FIELD_BYTES, MAX_FIELD_BYTES);
    assert_eq!(MAX_USER_BYTES, soma_guest::MAX_USER_BYTES);
    assert_eq!(MAX_STDIN_BYTES, soma_guest::MAX_STDIN_BYTES);
    assert_eq!(DirectCommand::GUEST_FIXED_BODY_BYTES, FIXED_BODY_SIZE);
    assert_eq!(DirectCommand::MAX_GUEST_BODY_BYTES, MAX_BODY_SIZE);

    // Slide the environment across the record boundary: the facade type and the guest
    // constructor must agree on every size, so a context one admits is one the other carries.
    let value = |bytes: usize| "v".repeat(bytes);
    for tail in 0..40_usize {
        let environment: Vec<(String, String)> = (0..15)
            .map(|index| (format!("K{index}"), value(MAX_FIELD_BYTES)))
            .chain([("TAIL".to_owned(), value(tail))])
            .collect();
        let stdin = vec![0_u8; 3_700 + tail * 90];
        let portable = CommandContext::new(environment.clone(), None, None, stdin.clone())
            .map_err(|_| ())
            .and_then(|context| {
                DirectCommand::new("/bin/true", Vec::<String>::new())
                    .map_err(|_| ())
                    .and_then(|command| command.with_context(context).map_err(|_| ()))
            });
        let guest = soma_guest::GuestCommand::new(
            b"/bin/true".to_vec(),
            Vec::new(),
            30_000,
            soma::ExecutionLimits::DEFAULT_MAX_OUTPUT_BYTES,
        )
        .and_then(|command| {
            command.with_context(soma_guest::CommandContext {
                environment: environment
                    .iter()
                    .map(|(n, v)| (n.as_bytes().to_vec(), v.as_bytes().to_vec()))
                    .collect(),
                working_directory: None,
                user: None,
                stdin,
            })
        });
        assert_eq!(portable.is_ok(), guest.is_ok(), "tail {tail}");
    }
}

#[test]
fn a_context_is_admitted_or_refused_before_a_machine_is_asked() {
    use soma::CommandContext;

    let context = CommandContext::new(
        vec![("A".to_owned(), "1".to_owned())],
        Some("/work".to_owned()),
        None,
        Vec::new(),
    )
    .expect("context");
    assert_eq!(admit_in("/bin/true", &[], 30_000, &context), Ok(()));

    // Fifteen full variables and a little more overflow one guest message, which only the
    // guest constructor can say once the context is attached.
    let big = CommandContext::new(
        (0..16)
            .map(|i| (format!("K{i}"), "v".repeat(MAX_FIELD_BYTES)))
            .collect(),
        None,
        None,
        vec![0; 4_000],
    )
    .expect("each bound holds");
    assert!(matches!(
        admit_in("/bin/true", &[], 30_000, &big),
        Err(Refusal::TooLargeTogether { .. })
    ));
}
