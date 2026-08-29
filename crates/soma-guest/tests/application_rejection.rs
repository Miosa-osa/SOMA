use soma_guest::{
    Error, GuestCommand, GuestMessage, HostMessage, MAX_RECORD_PAYLOAD, OperationId, OutputChunk,
    TerminalReport, TerminalStatus,
};

const HEADER_SIZE: usize = 28;

#[test]
fn malformed_application_headers_fail_closed_without_direction_confusion() {
    let valid = execute_message();
    let mut malformed = vec![Vec::new(), valid[..HEADER_SIZE - 1].to_vec()];
    malformed.push([valid.as_slice(), &[0]].concat());
    for offset in [0, 5, 6, 7, 8] {
        let mut message = valid.clone();
        message[offset] ^= 1;
        malformed.push(message);
    }
    let mut zero_operation = valid.clone();
    zero_operation[10..26].fill(0);
    malformed.push(zero_operation);
    for declared in [0_u16, u16::MAX] {
        let mut message = valid.clone();
        message[26..28].copy_from_slice(&declared.to_be_bytes());
        malformed.push(message);
    }
    malformed.push(vec![0; MAX_RECORD_PAYLOAD + 1]);

    for message in malformed {
        assert_eq!(
            HostMessage::decode(&message).expect_err("malformed header"),
            Error::ApplicationMessageRejected
        );
    }
    assert_eq!(
        GuestMessage::decode(&valid).expect_err("host message in guest direction"),
        Error::ApplicationMessageRejected
    );
    let guest = GuestMessage::repair_complete(operation())
        .encode()
        .expect("guest message");
    assert_eq!(
        HostMessage::decode(&guest).expect_err("guest message in host direction"),
        Error::ApplicationMessageRejected
    );
}

#[test]
fn hostile_command_bodies_are_rejected_before_use() {
    let valid = execute_message();
    let mut malformed = Vec::new();
    for range in [28..32, 32..40] {
        let mut message = valid.clone();
        message[range].fill(0);
        malformed.push(message);
    }
    let mut relative = valid.clone();
    relative[42] = b'x';
    malformed.push(relative);
    let mut nul = valid.clone();
    nul[43] = 0;
    malformed.push(nul);
    let mut too_many_arguments = valid.clone();
    too_many_arguments[51..53].copy_from_slice(&65_u16.to_be_bytes());
    malformed.push(too_many_arguments);
    let mut oversized_argument = valid.clone();
    oversized_argument[53..55].copy_from_slice(&4097_u16.to_be_bytes());
    malformed.push(oversized_argument);
    let mut trailing_body = valid.clone();
    trailing_body.push(0);
    let body_length = u16::try_from(trailing_body.len() - HEADER_SIZE).expect("bounded body");
    trailing_body[26..28].copy_from_slice(&body_length.to_be_bytes());
    malformed.push(trailing_body);

    for message in malformed {
        assert_eq!(
            HostMessage::decode(&message).expect_err("hostile command"),
            Error::ApplicationMessageRejected
        );
    }
}

#[test]
fn malformed_output_terminal_and_unit_bodies_are_rejected() {
    let valid_output =
        GuestMessage::stdout(operation(), OutputChunk::new(vec![1]).expect("output"))
            .encode()
            .expect("stdout");
    let mut empty_output = valid_output.clone();
    empty_output.truncate(HEADER_SIZE);
    empty_output[26..28].copy_from_slice(&0_u16.to_be_bytes());
    assert_rejected(&empty_output);
    let mut oversized_output = valid_output;
    oversized_output.resize(HEADER_SIZE + 4097, 1);
    oversized_output[26..28].copy_from_slice(&4097_u16.to_be_bytes());
    assert_rejected(&oversized_output);

    let valid_terminal = GuestMessage::terminal(
        operation(),
        TerminalReport::new(TerminalStatus::Exited(0), 0, 0).expect("terminal report"),
    )
    .encode()
    .expect("terminal");
    for (offset, value) in [(28, 0), (29, 1), (34, 1)] {
        let mut terminal = valid_terminal.clone();
        terminal[offset] = value;
        assert_rejected(&terminal);
    }
    let mut impossible_counts = valid_terminal;
    impossible_counts[36..40].copy_from_slice(&(16_u32 * 1024 * 1024 + 1).to_be_bytes());
    assert_rejected(&impossible_counts);

    for mut unit in [
        GuestMessage::repair_complete(operation())
            .encode()
            .expect("repair"),
        GuestMessage::shutdown_ack(operation())
            .encode()
            .expect("ack"),
    ] {
        unit.push(1);
        unit[26..28].copy_from_slice(&1_u16.to_be_bytes());
        assert_rejected(&unit);
    }
}

#[test]
fn shutdown_rejects_a_nonempty_body() {
    let mut shutdown = HostMessage::shutdown(operation())
        .encode()
        .expect("canonical shutdown");
    shutdown.push(1);
    shutdown[26..28].copy_from_slice(&1_u16.to_be_bytes());

    assert_eq!(
        HostMessage::decode(&shutdown).expect_err("nonempty Shutdown body"),
        Error::ApplicationMessageRejected
    );
}

fn execute_message() -> Vec<u8> {
    HostMessage::execute(
        operation(),
        GuestCommand::new(
            b"/bin/echo".to_vec(),
            vec![b"hi".to_vec(), Vec::new()],
            1_000,
            4_096,
        )
        .expect("command"),
    )
    .encode()
    .expect("execute")
}

fn operation() -> OperationId {
    OperationId::new([9; 16]).expect("operation")
}

fn assert_rejected(message: &[u8]) {
    assert_eq!(
        GuestMessage::decode(message).expect_err("hostile guest message"),
        Error::ApplicationMessageRejected
    );
}
