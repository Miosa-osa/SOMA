use soma_guest::{
    Error, GuestCommand, HostMessage, MAX_ARGUMENT_BYTES, MAX_RECORD_PAYLOAD, OperationId,
    OutputChunk, TerminalReport, TerminalStatus,
};

#[test]
fn largest_command_fits_one_record_and_one_more_byte_is_rejected() {
    let mut arguments = vec![vec![b'x'; 4096]; 15];
    arguments.push(vec![b'y'; 3985]);
    let command =
        GuestCommand::new(b"/p".to_vec(), arguments.clone(), 1, 1).expect("exact record boundary");
    let message = HostMessage::execute(OperationId::new([1; 16]).expect("operation"), command);
    assert_eq!(
        message.encode().expect("maximum message").len(),
        MAX_RECORD_PAYLOAD
    );

    arguments[15].push(b'y');
    assert_eq!(
        GuestCommand::new(b"/p".to_vec(), arguments, 1, 1).expect_err("one byte too large"),
        Error::InvalidCommand
    );
}

#[test]
fn one_argument_is_bounded_exactly_and_a_maximal_one_round_trips_in_one_record() {
    // A shell script is one argument of `sh -lc`; a real client sends 12 KB that way.
    for bytes in [MAX_ARGUMENT_BYTES - 1, MAX_ARGUMENT_BYTES] {
        let command = GuestCommand::new(
            b"/bin/sh".to_vec(),
            vec![b"-lc".to_vec(), vec![b'a'; bytes]],
            1,
            1,
        )
        .expect("an argument within its bound");
        let message = HostMessage::execute(
            OperationId::new([1; 16]).expect("operation"),
            command.clone(),
        );
        let encoded = message.encode().expect("one record");
        assert!(encoded.len() <= MAX_RECORD_PAYLOAD);
        assert_eq!(
            HostMessage::decode(&encoded).expect("one canonical round trip"),
            message,
            "{bytes}"
        );
    }
    assert_eq!(
        GuestCommand::new(
            b"/bin/sh".to_vec(),
            vec![b"-lc".to_vec(), vec![b'a'; MAX_ARGUMENT_BYTES + 1]],
            1,
            1,
        )
        .expect_err("one byte past the argument bound"),
        Error::InvalidCommand
    );
}

#[test]
fn a_decoder_refuses_a_record_carrying_one_argument_byte_past_the_bound() {
    // A record a host built by hand, so the bound is the only thing that can refuse it: the
    // argument's bytes are all there and the record is far below its own size limit.
    for (bytes, admitted) in [(MAX_ARGUMENT_BYTES, true), (MAX_ARGUMENT_BYTES + 1, false)] {
        let record = execute_record(&vec![b'a'; bytes]);
        assert!(record.len() < MAX_RECORD_PAYLOAD);
        let decoded = HostMessage::decode(&record);
        if admitted {
            decoded.expect("an argument at the bound");
        } else {
            assert_eq!(
                decoded.expect_err("an argument past the bound"),
                Error::ApplicationMessageRejected
            );
        }
    }
}

/// One `Execute` record for `/p` with one argument, written out field by field.
fn execute_record(argument: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&1_u32.to_be_bytes());
    body.extend_from_slice(&1_u64.to_be_bytes());
    body.extend_from_slice(&2_u16.to_be_bytes());
    body.extend_from_slice(b"/p");
    body.extend_from_slice(&1_u16.to_be_bytes());
    body.extend_from_slice(
        &u16::try_from(argument.len())
            .expect("a u16 length")
            .to_be_bytes(),
    );
    body.extend_from_slice(argument);
    // No environment, no directory, no user, no standard input.
    body.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    let mut record = b"SOMA".to_vec();
    record.extend_from_slice(&1_u16.to_be_bytes());
    record.extend_from_slice(&[2, 0, 0, 0]);
    record.extend_from_slice(&[1; 16]);
    record.extend_from_slice(
        &u16::try_from(body.len())
            .expect("a u16 length")
            .to_be_bytes(),
    );
    record.extend_from_slice(&body);
    record
}

#[test]
fn command_constructor_rejects_every_unrepresentable_direct_invocation() {
    let invalid = [
        GuestCommand::new(Vec::new(), vec![], 1, 1),
        GuestCommand::new(b"relative".to_vec(), vec![], 1, 1),
        GuestCommand::new(b"/bad\0path".to_vec(), vec![], 1, 1),
        GuestCommand::new([b"/".as_slice(), &vec![b'p'; 4096]].concat(), vec![], 1, 1),
        GuestCommand::new(b"/p".to_vec(), vec![vec![]; 65], 1, 1),
        GuestCommand::new(b"/p".to_vec(), vec![b"bad\0arg".to_vec()], 1, 1),
        GuestCommand::new(
            b"/p".to_vec(),
            vec![vec![b'a'; MAX_ARGUMENT_BYTES + 1]],
            1,
            1,
        ),
        GuestCommand::new(b"/p".to_vec(), vec![], 0, 1),
        GuestCommand::new(b"/p".to_vec(), vec![], 3_600_001, 1),
        GuestCommand::new(b"/p".to_vec(), vec![], 1, 0),
        GuestCommand::new(b"/p".to_vec(), vec![], 1, 16 * 1024 * 1024 + 1),
    ];

    for result in invalid {
        assert_eq!(result.expect_err("invalid command"), Error::InvalidCommand);
    }
    assert_eq!(OperationId::new([0; 16]), Err(Error::InvalidOperation));
}

#[test]
fn output_and_terminal_constructors_enforce_stream_bounds() {
    assert_eq!(
        OutputChunk::new(Vec::new()).expect_err("empty output"),
        Error::InvalidOutputChunk
    );
    assert_eq!(
        OutputChunk::new(vec![1; 4097]).expect_err("oversized output"),
        Error::InvalidOutputChunk
    );
    for status in [
        TerminalStatus::Exited(-1),
        TerminalStatus::Exited(256),
        TerminalStatus::Signaled(0),
        TerminalStatus::Signaled(65),
        TerminalStatus::ExecFailed(0),
        TerminalStatus::ExecFailed(4096),
        TerminalStatus::AgentFailed(0),
        TerminalStatus::AgentFailed(-1),
        TerminalStatus::AgentFailed(4096),
    ] {
        assert_eq!(
            TerminalReport::new(status, 0, 0).expect_err("invalid terminal"),
            Error::InvalidTerminalStatus
        );
    }
    assert_eq!(
        TerminalReport::new(TerminalStatus::Exited(0), 16 * 1024 * 1024, 1)
            .expect_err("terminal counts exceed protocol allowance"),
        Error::InvalidTerminalReport
    );
    let maximum = TerminalReport::new(TerminalStatus::OutputLimit, 16 * 1024 * 1024, 0)
        .expect("maximum output count");
    assert_eq!(maximum.stdout_bytes(), 16 * 1024 * 1024);
    assert_eq!(maximum.stderr_bytes(), 0);
    assert_eq!(maximum.status(), TerminalStatus::OutputLimit);
}
