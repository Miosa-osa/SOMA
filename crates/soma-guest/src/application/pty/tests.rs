//! What the terminal wire form must accept, and what it must refuse.

use super::super::{GuestMessage, HostMessage, OperationId};
use super::{
    MAX_PTY_CHUNK_BYTES, MAX_PTY_COLUMNS, MAX_PTY_ROWS, MAX_PTY_WAIT_MILLIS, PtyFailure,
    PtyOutcome, PtyRequest, PtySize,
};

fn operation() -> OperationId {
    OperationId::new([9; 16]).expect("a non-zero identity")
}

fn size(columns: u16, rows: u16) -> PtySize {
    PtySize::new(columns, rows).expect("bounded dimensions")
}

/// Every request survives its own encoding exactly.
#[test]
fn every_request_round_trips() {
    for request in [
        PtyRequest::Open(size(80, 24)),
        PtyRequest::Write {
            bytes: b"echo hello\n".to_vec().into(),
        },
        PtyRequest::Write {
            bytes: vec![0; MAX_PTY_CHUNK_BYTES].into(),
        },
        PtyRequest::Read { wait_millis: 0 },
        PtyRequest::Read {
            wait_millis: MAX_PTY_WAIT_MILLIS,
        },
        PtyRequest::Resize(size(MAX_PTY_COLUMNS, MAX_PTY_ROWS)),
        PtyRequest::Close,
    ] {
        let message = HostMessage::pty(operation(), request);
        let encoded = message.encode().expect("a bounded request fits one record");
        assert_eq!(HostMessage::decode(&encoded).as_ref(), Ok(&message));
    }
}

/// Every outcome survives its own encoding exactly.
#[test]
fn every_outcome_round_trips() {
    for outcome in [
        PtyOutcome::Opened(size(120, 40)),
        PtyOutcome::Wrote { bytes: 11 },
        PtyOutcome::Output {
            bytes: b"$ ".to_vec().into(),
            end: false,
        },
        PtyOutcome::Output {
            bytes: Box::default(),
            end: true,
        },
        PtyOutcome::Resized(size(1, 1)),
        PtyOutcome::Closed,
        PtyOutcome::Failed(PtyFailure::NoSession),
        PtyOutcome::Failed(PtyFailure::AlreadyOpen),
        PtyOutcome::Failed(PtyFailure::Denied),
        PtyOutcome::Failed(PtyFailure::Failed),
    ] {
        let message = GuestMessage::pty_outcome(operation(), outcome);
        let encoded = message.encode().expect("a bounded outcome fits one record");
        assert_eq!(GuestMessage::decode(&encoded).as_ref(), Ok(&message));
    }
}

/// A terminal with no cells, or more cells than any screen has, is not a request.
#[test]
fn an_inadmissible_dimension_is_refused() {
    for (columns, rows) in [
        (0, 24),
        (80, 0),
        (0, 0),
        (MAX_PTY_COLUMNS + 1, 24),
        (80, MAX_PTY_ROWS + 1),
        (u16::MAX, u16::MAX),
    ] {
        assert!(
            PtySize::new(columns, rows).is_err(),
            "{columns}x{rows} was accepted"
        );
    }
}

/// The decoder refuses the dimensions the constructor refuses, not just the constructor.
#[test]
fn an_inadmissible_dimension_on_the_wire_is_refused() {
    let mut encoded = HostMessage::pty(operation(), PtyRequest::Open(size(80, 24)))
        .encode()
        .expect("encodes");
    let rows = encoded.len() - 2;
    let columns = encoded.len() - 4;
    encoded[rows] = 0;
    encoded[rows + 1] = 0;
    assert!(
        HostMessage::decode(&encoded).is_err(),
        "a terminal with no rows decoded"
    );

    encoded[rows + 1] = 24;
    encoded[columns] = 0xFF;
    encoded[columns + 1] = 0xFF;
    assert!(
        HostMessage::decode(&encoded).is_err(),
        "a terminal of 65535 columns decoded"
    );
}

/// A chunk larger than one record's share is refused at the decoder.
#[test]
fn an_oversized_chunk_is_refused() {
    let request = PtyRequest::Write {
        bytes: vec![b'x'; MAX_PTY_CHUNK_BYTES + 1].into(),
    };
    let encoded = HostMessage::pty(operation(), request)
        .encode()
        .expect("the record still holds it");
    assert!(
        HostMessage::decode(&encoded).is_err(),
        "an oversized write chunk decoded"
    );

    let outcome = PtyOutcome::Output {
        bytes: vec![b'x'; MAX_PTY_CHUNK_BYTES + 1].into(),
        end: false,
    };
    let encoded = GuestMessage::pty_outcome(operation(), outcome)
        .encode()
        .expect("the record still holds it");
    assert!(
        GuestMessage::decode(&encoded).is_err(),
        "an oversized output chunk decoded"
    );
}

/// A read that could wait longer than the bound would hold the one channel open.
#[test]
fn an_unbounded_wait_is_refused() {
    let mut encoded = HostMessage::pty(
        operation(),
        PtyRequest::Read {
            wait_millis: MAX_PTY_WAIT_MILLIS,
        },
    )
    .encode()
    .expect("encodes");
    let last = encoded.len() - 4;
    encoded[last..].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(
        HostMessage::decode(&encoded).is_err(),
        "an unbounded wait decoded"
    );
}

/// Asking anything of a session that is not open is refused, and the refusal names why.
#[test]
fn a_request_against_an_unknown_session_is_refused_on_the_wire() {
    let message = GuestMessage::pty_outcome(operation(), PtyOutcome::Failed(PtyFailure::NoSession));
    let encoded = message.encode().expect("encodes");
    assert_eq!(GuestMessage::decode(&encoded).as_ref(), Ok(&message));
    assert_eq!(format!("{}", PtyFailure::NoSession), "no terminal session");
}

/// A boolean is exactly zero or one, so one message has exactly one encoding.
#[test]
fn a_boolean_that_is_neither_zero_nor_one_is_refused() {
    let outcome = PtyOutcome::Output {
        bytes: Box::default(),
        end: false,
    };
    let mut encoded = GuestMessage::pty_outcome(operation(), outcome)
        .encode()
        .expect("encodes");
    let end = encoded.len() - 3;
    encoded[end] = 2;
    assert!(
        GuestMessage::decode(&encoded).is_err(),
        "an end flag of two decoded"
    );
}

/// A trailing byte is a different message, so it is refused rather than ignored.
#[test]
fn a_trailing_byte_is_refused() {
    let mut encoded = HostMessage::pty(operation(), PtyRequest::Close)
        .encode()
        .expect("encodes");
    encoded.push(0);
    assert!(
        HostMessage::decode(&encoded).is_err(),
        "a trailing byte decoded"
    );
}

/// An unknown discriminant is a message this version does not have.
#[test]
fn an_unknown_discriminant_is_refused() {
    let mut encoded = HostMessage::pty(operation(), PtyRequest::Close)
        .encode()
        .expect("encodes");
    let discriminant = encoded.len() - 1;
    encoded[discriminant] = 200;
    assert!(
        HostMessage::decode(&encoded).is_err(),
        "an unknown request discriminant decoded"
    );
}

/// Neither what is typed at a terminal nor what it prints may reach a log through a formatter.
#[test]
fn debug_reports_shapes_and_never_terminal_bytes() {
    let request = PtyRequest::Write {
        bytes: b"export TOKEN=hunter2\n".to_vec().into(),
    };
    let rendered = format!("{request:?}");
    assert!(!rendered.contains("hunter2"), "{rendered}");
    assert!(rendered.contains("21 bytes"), "{rendered}");

    let outcome = PtyOutcome::Output {
        bytes: b"root@sandbox:~# hunter2".to_vec().into(),
        end: false,
    };
    let rendered = format!("{outcome:?}");
    assert!(!rendered.contains("hunter2"), "{rendered}");
    assert!(!rendered.contains("root@sandbox"), "{rendered}");
    assert!(rendered.contains("23 bytes"), "{rendered}");

    // The whole message must be as quiet as the parts, since that is what a caller logs.
    let message = HostMessage::pty(
        operation(),
        PtyRequest::Write {
            bytes: b"hunter2".to_vec().into(),
        },
    );
    assert!(!format!("{message:?}").contains("hunter2"));
}
