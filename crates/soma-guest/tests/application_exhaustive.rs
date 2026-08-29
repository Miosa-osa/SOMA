use soma_guest::{Error, GuestCommand, HostMessage, OperationId};

const HEADER_SIZE: usize = 28;

#[test]
fn every_application_body_length_declaration_is_exact() {
    let mut encoded = execute_message();
    let expected = u16::try_from(encoded.len() - HEADER_SIZE).expect("bounded fixture");
    for declared in 0..=u16::MAX {
        encoded[26..28].copy_from_slice(&declared.to_be_bytes());
        let result = HostMessage::decode(&encoded);
        assert_eq!(result.is_ok(), declared == expected, "declared {declared}");
    }
}

#[test]
fn every_command_field_and_count_declaration_is_bounded_and_exact() {
    exhaustive_field(40, 2);
    exhaustive_field(44, 1);
    exhaustive_field(46, 1);
}

fn exhaustive_field(offset: usize, expected: u16) {
    let mut encoded = execute_message();
    for declared in 0..=u16::MAX {
        encoded[offset..offset + 2].copy_from_slice(&declared.to_be_bytes());
        let result = HostMessage::decode(&encoded);
        if declared == expected {
            assert!(result.is_ok(), "canonical declaration {declared}");
        } else {
            assert_eq!(
                result.expect_err("noncanonical declaration"),
                Error::ApplicationMessageRejected,
                "declared {declared} at {offset}"
            );
        }
    }
}

fn execute_message() -> Vec<u8> {
    HostMessage::execute(
        OperationId::new([1; 16]).expect("operation"),
        GuestCommand::new(b"/p".to_vec(), vec![b"x".to_vec()], 1, 1).expect("command"),
    )
    .encode()
    .expect("message")
}
