//! The portable facade's terminal rules must be the guest protocol's terminal rules.
//!
//! `soma` sits below `soma-guest` and cannot call into it, so it restates the bounds a surface
//! needs in order to refuse an inadmissible terminal call before it reaches the wire. This crate
//! depends on both, which makes it the one place the two can be compared. If they ever disagree,
//! a call one accepts and the other rejects would arrive at the guest as a protocol fault and end
//! the session, destroying the caller's sandbox instead of telling it no.

use soma::PtyOperation;

/// Every terminal call either both admit or both refuse.
#[test]
fn the_facade_and_the_protocol_admit_exactly_the_same_terminal_calls() {
    let cases = vec![
        PtyOperation::Open {
            columns: 80,
            rows: 24,
        },
        PtyOperation::Open {
            columns: 0,
            rows: 24,
        },
        PtyOperation::Open {
            columns: 80,
            rows: 0,
        },
        PtyOperation::Open {
            columns: soma::MAX_PTY_COLUMNS,
            rows: soma::MAX_PTY_ROWS,
        },
        PtyOperation::Open {
            columns: soma::MAX_PTY_COLUMNS + 1,
            rows: soma::MAX_PTY_ROWS,
        },
        PtyOperation::Resize {
            columns: 120,
            rows: 40,
        },
        PtyOperation::Resize {
            columns: 120,
            rows: soma::MAX_PTY_ROWS + 1,
        },
        PtyOperation::Write {
            bytes: vec![0xff; soma::MAX_PTY_CHUNK_BYTES],
        },
        PtyOperation::Write {
            bytes: vec![0xff; soma::MAX_PTY_CHUNK_BYTES + 1],
        },
        PtyOperation::Read { wait_millis: 0 },
        PtyOperation::Read {
            wait_millis: soma::MAX_PTY_WAIT_MILLIS,
        },
        PtyOperation::Read {
            wait_millis: soma::MAX_PTY_WAIT_MILLIS + 1,
        },
        PtyOperation::Close,
    ];

    for operation in cases {
        let facade = operation.check().is_ok();
        // The protocol's own check is reached by asking it to encode and decode the request,
        // which is the exact path a real call takes, so nothing here can drift from it.
        let protocol = guest_request(&operation).is_some_and(|request| {
            soma_guest::PtyRequest::decode_body(&request.encode_body()).is_ok()
        });
        assert_eq!(facade, protocol, "the two disagree about {operation:?}");
    }
}

/// The restated bounds are the protocol's own bounds, not merely compatible with them.
#[test]
fn the_restated_terminal_bounds_are_the_protocol_bounds() {
    assert_eq!(soma::MAX_PTY_CHUNK_BYTES, soma_guest::MAX_PTY_CHUNK_BYTES);
    assert_eq!(soma::MAX_PTY_COLUMNS, soma_guest::MAX_PTY_COLUMNS);
    assert_eq!(soma::MAX_PTY_ROWS, soma_guest::MAX_PTY_ROWS);
    assert_eq!(soma::MAX_PTY_WAIT_MILLIS, soma_guest::MAX_PTY_WAIT_MILLIS);
}

/// The mapping under test, restated here because it is private to the backend.
///
/// A size the protocol will not carry produces nothing rather than a clamped request, which is
/// what the comparison above is checking: the facade must refuse exactly those.
fn guest_request(operation: &PtyOperation) -> Option<soma_guest::PtyRequest> {
    Some(match operation {
        PtyOperation::Open { columns, rows } => {
            soma_guest::PtyRequest::Open(soma_guest::PtySize::new(*columns, *rows).ok()?)
        }
        PtyOperation::Resize { columns, rows } => {
            soma_guest::PtyRequest::Resize(soma_guest::PtySize::new(*columns, *rows).ok()?)
        }
        PtyOperation::Write { bytes } => soma_guest::PtyRequest::Write {
            bytes: bytes.as_slice().into(),
        },
        PtyOperation::Read { wait_millis } => soma_guest::PtyRequest::Read {
            wait_millis: *wait_millis,
        },
        PtyOperation::Close => soma_guest::PtyRequest::Close,
    })
}
