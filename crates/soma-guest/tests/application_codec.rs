use soma_guest::{
    GuestCommand, GuestMessage, HostMessage, OperationId, OutputChunk, TerminalReport,
    TerminalStatus,
};

#[test]
fn one_execute_message_has_one_canonical_application_round_trip() {
    let message = HostMessage::execute(
        OperationId::new([7; 16]).expect("operation"),
        GuestCommand::new(
            b"/usr/local/bin/node".to_vec(),
            vec![b"--version".to_vec(), Vec::new(), b"a b".to_vec()],
            1_000,
            1_024,
        )
        .expect("bounded command"),
    );
    let encoded = message.encode().expect("canonical message");
    assert_eq!(
        HostMessage::decode(&encoded).expect("decoded message"),
        message
    );
}

#[test]
fn every_typed_application_message_has_one_canonical_round_trip() {
    let operation = OperationId::new([8; 16]).expect("operation");
    let command = GuestCommand::new(b"/bin/true".to_vec(), vec![], 3_600_000, 16 * 1024 * 1024)
        .expect("command");
    let host_messages = [
        HostMessage::prepare_and_probe(operation),
        HostMessage::execute(operation, command),
        HostMessage::shutdown(operation),
    ];
    for message in host_messages {
        let encoded = message.encode().expect("host message");
        assert_eq!(
            HostMessage::decode(&encoded).expect("host round trip"),
            message
        );
    }

    let guest_messages = [
        GuestMessage::repair_complete(operation),
        GuestMessage::stdout(
            operation,
            OutputChunk::new(b"out\0binary".to_vec()).expect("stdout"),
        ),
        GuestMessage::stderr(
            operation,
            OutputChunk::new(b"error".to_vec()).expect("stderr"),
        ),
        terminal(operation, TerminalStatus::Exited(17)),
        terminal(operation, TerminalStatus::Signaled(9)),
        terminal(operation, TerminalStatus::TimedOut),
        terminal(operation, TerminalStatus::OutputLimit),
        terminal(operation, TerminalStatus::ExecFailed(2)),
        terminal(operation, TerminalStatus::AgentFailed(1)),
        GuestMessage::shutdown_ack(operation),
    ];
    for message in guest_messages {
        let encoded = message.encode().expect("guest message");
        assert_eq!(
            GuestMessage::decode(&encoded).expect("guest round trip"),
            message
        );
    }
}

fn terminal(operation: OperationId, status: TerminalStatus) -> GuestMessage {
    GuestMessage::terminal(
        operation,
        TerminalReport::new(status, 3, 5).expect("terminal report"),
    )
}

#[test]
fn execute_message_matches_the_frozen_v1_vector() {
    let message = HostMessage::execute(
        OperationId::new([0x11; 16]).expect("operation"),
        GuestCommand::new(
            b"/bin/echo".to_vec(),
            vec![b"hi".to_vec(), Vec::new()],
            1_000,
            4_096,
        )
        .expect("command"),
    );
    let expected = decode_hex(concat!(
        "534f4d4100010200000011111111111111111111111111111111",
        "001f000003e8000000000000100000092f62696e2f6563686f0002000268690000",
    ));

    assert_eq!(message.encode().expect("canonical vector"), expected);
    assert_eq!(
        HostMessage::decode(&expected).expect("frozen vector"),
        message
    );
}

#[test]
fn terminal_report_matches_the_frozen_v1_vector() {
    let message = GuestMessage::terminal(
        OperationId::new([0x22; 16]).expect("operation"),
        TerminalReport::new(TerminalStatus::OutputLimit, 3, 5).expect("terminal report"),
    );
    let expected = decode_hex(concat!(
        "534f4d4100018400000022222222222222222222222222222222",
        "001004000000000000000000000300000005",
    ));

    assert_eq!(message.encode().expect("canonical vector"), expected);
    assert_eq!(
        GuestMessage::decode(&expected).expect("frozen vector"),
        message
    );
}

fn decode_hex(encoded: &str) -> Vec<u8> {
    let (pairs, remainder) = encoded.as_bytes().as_chunks::<2>();
    assert!(remainder.is_empty());
    pairs
        .iter()
        .map(|pair| {
            let text = core::str::from_utf8(pair).expect("ASCII hex");
            u8::from_str_radix(text, 16).expect("valid hex")
        })
        .collect()
}
