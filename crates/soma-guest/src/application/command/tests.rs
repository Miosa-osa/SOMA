use super::super::MAX_BODY_SIZE;
use super::{CommandContext, GuestCommand, MAX_FIELD_BYTES, MAX_STDIN_BYTES};
use crate::Error;

fn command() -> GuestCommand {
    GuestCommand::new(
        b"/bin/build".to_vec(),
        vec![b"--release".to_vec()],
        1_000,
        64,
    )
    .expect("bounded command")
}

fn round_trip(command: &GuestCommand) -> GuestCommand {
    GuestCommand::decode_body(&command.encode_body()).expect("one canonical round trip")
}

#[test]
fn every_context_field_survives_one_round_trip_exactly() {
    let context = CommandContext {
        environment: vec![
            (b"TOKEN".to_vec(), b"s3cret".to_vec()),
            (b"EMPTY".to_vec(), Vec::new()),
            (b"BINARY".to_vec(), vec![0xff, 0xfe]),
        ],
        working_directory: Some(b"/work/space".to_vec()),
        user: Some(b"builder".to_vec()),
        stdin: b"line one\nline two\n".to_vec(),
    };
    let command = command().with_context(context).expect("bounded context");

    let decoded = round_trip(&command);

    assert_eq!(decoded, command);
    assert_eq!(decoded.environment().len(), 3);
    assert_eq!(&*decoded.environment()[0].0, b"TOKEN");
    assert_eq!(&*decoded.environment()[0].1, b"s3cret");
    assert_eq!(&*decoded.environment()[1].1, b"");
    assert_eq!(&*decoded.environment()[2].1, [0xff, 0xfe]);
    assert_eq!(decoded.working_directory(), Some(b"/work/space".as_slice()));
    assert_eq!(decoded.user(), Some(b"builder".as_slice()));
    assert_eq!(decoded.stdin(), b"line one\nline two\n");
}

#[test]
fn an_absent_context_round_trips_as_absent_rather_than_empty() {
    let command = command();

    let decoded = round_trip(&command);

    assert_eq!(decoded, command);
    assert!(decoded.environment().is_empty());
    assert_eq!(decoded.working_directory(), None);
    assert_eq!(decoded.user(), None);
    assert!(decoded.stdin().is_empty());
}

#[test]
fn the_context_constructor_refuses_every_unrepresentable_field() {
    let refused = [
        // An environment name may not be empty, carry the separator, carry a nul, or be
        // longer than one field.
        environment(Vec::new(), b"v".to_vec()),
        environment(b"NAME=EXTRA".to_vec(), b"v".to_vec()),
        environment(b"NA\0ME".to_vec(), b"v".to_vec()),
        environment(vec![b'n'; 4097], b"v".to_vec()),
        environment(b"NAME".to_vec(), b"va\0lue".to_vec()),
        environment(b"NAME".to_vec(), vec![b'v'; 4097]),
        CommandContext {
            environment: vec![(b"N".to_vec(), b"v".to_vec()); 65],
            ..CommandContext::default()
        },
        working_directory(Vec::new()),
        working_directory(b"relative/path".to_vec()),
        working_directory(b"/bad\0path".to_vec()),
        working_directory([b"/".as_slice(), &vec![b'p'; 4096]].concat()),
        user(Vec::new()),
        user(b"na\0me".to_vec()),
        user(vec![b'u'; 257]),
        CommandContext {
            stdin: vec![0; MAX_STDIN_BYTES + 1],
            ..CommandContext::default()
        },
    ];

    for context in refused {
        assert_eq!(
            command()
                .with_context(context)
                .expect_err("refused context"),
            Error::InvalidCommand
        );
    }
}

#[test]
fn the_largest_context_fits_one_record_and_one_more_byte_is_refused() {
    let room = MAX_BODY_SIZE - command().encoded_size() - MAX_STDIN_BYTES;
    let context = |room: usize| CommandContext {
        environment: filler(room),
        stdin: vec![b's'; MAX_STDIN_BYTES],
        ..CommandContext::default()
    };
    let exact = command()
        .with_context(context(room))
        .expect("exact record boundary");

    assert_eq!(exact.encoded_size(), MAX_BODY_SIZE);
    assert_eq!(round_trip(&exact), exact);

    assert_eq!(
        command()
            .with_context(context(room + 1))
            .expect_err("one byte too large"),
        Error::InvalidCommand
    );
}

/// Builds environment pairs whose encoded bytes total exactly `bytes`.
///
/// One pair cannot fill a whole record because a value is bounded on its own, so the aggregate
/// bound is only reachable by several of them, which is precisely the case worth testing.
fn filler(bytes: usize) -> Vec<(Vec<u8>, Vec<u8>)> {
    // A pair costs a one-byte name and the two length prefixes on top of its value.
    const OVERHEAD: usize = 2 + 1 + 2;
    let mut pairs = Vec::new();
    let mut remaining = bytes;
    while remaining > 0 {
        let value = (remaining - OVERHEAD).min(MAX_FIELD_BYTES);
        pairs.push((b"F".to_vec(), vec![b'v'; value]));
        remaining -= OVERHEAD + value;
    }
    pairs
}

#[test]
fn a_decoder_refuses_a_second_encoding_of_one_optional_field() {
    let present = command()
        .with_context(working_directory(b"/w".to_vec()))
        .expect("bounded context");
    let body = present.encode_body();
    // The tail is the working-directory flag, its length and bytes, the absent-user flag, and
    // the two-byte length of the empty standard input.
    let flag = body.len() - (1 + 2 + b"/w".len() + 1 + 2);
    assert_eq!(body[flag], 1);

    for corrupted in [2, 255] {
        let mut body = body.clone();
        body[flag] = corrupted;
        assert_eq!(
            GuestCommand::decode_body(&body).expect_err("a flag that is neither zero nor one"),
            Error::ApplicationMessageRejected
        );
    }

    // Present with an empty field would be a second spelling of absent.
    let mut empty = body.clone();
    empty.drain(flag + 1..flag + 3 + b"/w".len());
    empty.splice((flag + 1)..=flag, [0, 0]);
    assert_eq!(
        GuestCommand::decode_body(&empty).expect_err("a present but empty field"),
        Error::ApplicationMessageRejected
    );
}

#[test]
fn debug_never_prints_an_environment_value_a_working_directory_or_standard_input() {
    let context = CommandContext {
        environment: vec![(b"TOKEN".to_vec(), b"hunter2-do-not-log".to_vec())],
        working_directory: Some(b"/tenants/acme/checkout".to_vec()),
        user: Some(b"builder".to_vec()),
        stdin: b"private-input-do-not-log".to_vec(),
    };
    let command = command()
        .with_context(context.clone())
        .expect("bounded context");

    for rendered in [format!("{command:?}"), format!("{context:?}")] {
        assert!(!rendered.contains("hunter2-do-not-log"), "{rendered}");
        assert!(!rendered.contains("/tenants/acme/checkout"), "{rendered}");
        assert!(!rendered.contains("private-input-do-not-log"), "{rendered}");
        assert!(!rendered.contains("builder"), "{rendered}");
        assert!(rendered.contains("environment_count: 1"), "{rendered}");
        assert!(rendered.contains("stdin_bytes: 24"), "{rendered}");
    }
}

fn environment(name: Vec<u8>, value: Vec<u8>) -> CommandContext {
    CommandContext {
        environment: vec![(name, value)],
        ..CommandContext::default()
    }
}

fn working_directory(path: Vec<u8>) -> CommandContext {
    CommandContext {
        working_directory: Some(path),
        ..CommandContext::default()
    }
}

fn user(name: Vec<u8>) -> CommandContext {
    CommandContext {
        user: Some(name),
        ..CommandContext::default()
    }
}
