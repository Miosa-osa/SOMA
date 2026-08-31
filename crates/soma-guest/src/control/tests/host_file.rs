use std::thread;

use crate::{
    ControlError, ControlFailureClass, ControlStage, FileOutcome, FileRequest, GuestMessage,
    HostControl, HostMessage, MAX_CHUNK_BYTES, OperationId, TerminalReport, TerminalStatus,
};

use super::super::{RepairedHostControl, WholeFileRead, WholeFileWrite};
use super::support::{MemoryIo, Observation, RawGuest, launch, pair};

type Host = HostControl<MemoryIo>;
pub(super) type Repaired = RepairedHostControl<MemoryIo>;

#[test]
fn an_answer_under_another_operation_is_rejected() {
    let (host, mut raw, observed) = repaired_host();
    let host_thread = thread::spawn(move || host.file(exists()));
    let HostMessage::File { .. } = raw.receive() else {
        panic!("the host sends a file request");
    };
    raw.send(GuestMessage::file_outcome(
        operation(9),
        FileOutcome::Status { kind: None },
    ));

    let error = control_error(host_thread.join().expect("host thread"));
    assert_eq!(error.stage(), ControlStage::File);
    assert_eq!(error.class(), ControlFailureClass::Protocol);
    assert_eq!(observed.poison(), 1);
}

#[test]
fn an_answer_of_another_kind_is_rejected() {
    let (host, mut raw, observed) = repaired_host();
    let host_thread = thread::spawn(move || host.file(exists()));
    let HostMessage::File { operation, .. } = raw.receive() else {
        panic!("the host sends a file request");
    };
    raw.send(GuestMessage::terminal(
        operation,
        TerminalReport::new(TerminalStatus::Exited(0), 0, 0).expect("report"),
    ));

    let error = control_error(host_thread.join().expect("host thread"));
    assert_eq!(error.stage(), ControlStage::File);
    assert_eq!(error.class(), ControlFailureClass::Lifecycle);
    assert_eq!(observed.poison(), 1);
}

#[test]
fn one_request_returns_its_answer_and_keeps_the_session() {
    let (host, mut raw, observed) = repaired_host();
    let host_thread = thread::spawn(move || {
        let (host, first) = host.file(exists()).expect("first request");
        let (host, second) = host.file(exists()).expect("second request");
        drop(host);
        (first, second)
    });
    let mut seen = Vec::new();
    for _ in 0..2 {
        let HostMessage::File { operation, request } = raw.receive() else {
            panic!("the host sends a file request");
        };
        assert!(matches!(request, FileRequest::Exists { .. }));
        seen.push(operation);
        raw.send(GuestMessage::file_outcome(
            operation,
            FileOutcome::Status { kind: None },
        ));
    }

    let (first, second) = host_thread.join().expect("host thread");
    assert_eq!(first, FileOutcome::Status { kind: None });
    assert_eq!(second, FileOutcome::Status { kind: None });
    assert_ne!(seen[0], seen[1], "each request mints its own identity");
    assert_eq!(observed.poison(), 0);
}

#[test]
fn reading_a_whole_file_assembles_every_chunk() {
    let contents = body(MAX_CHUNK_BYTES + 17);
    let expected = contents.clone();
    let (host, mut raw, observed) = repaired_host();
    let host_thread =
        thread::spawn(move || host.read_whole_file(b"/tmp/file", MAX_CHUNK_BYTES * 4));
    let mut offsets = Vec::new();
    let mut served = 0;
    while served < contents.len() {
        let HostMessage::File { operation, request } = raw.receive() else {
            panic!("the host sends a file request");
        };
        let FileRequest::Read { offset, length, .. } = request else {
            panic!("a whole-file read asks for reads");
        };
        offsets.push(offset);
        let wanted = usize::try_from(length).expect("a bounded length");
        let end = (served + wanted).min(contents.len());
        raw.send(GuestMessage::file_outcome(
            operation,
            FileOutcome::Read {
                bytes: contents[served..end].into(),
                end: end == contents.len(),
            },
        ));
        served = end;
    }

    let (_host, outcome) = host_thread.join().expect("host thread").expect("read");
    assert_eq!(outcome, WholeFileRead::Bytes(expected));
    assert_eq!(
        offsets,
        vec![0, u64::try_from(MAX_CHUNK_BYTES).expect("an offset")]
    );
    assert_eq!(observed.poison(), 0);
}

#[test]
fn writing_a_whole_file_sends_every_chunk_and_shortens_once() {
    let contents = body(MAX_CHUNK_BYTES + 17);
    let sent = contents.clone();
    let (host, mut raw, observed) = repaired_host();
    let host_thread =
        thread::spawn(move || host.write_whole_file(b"/tmp/file", &sent, MAX_CHUNK_BYTES * 4));
    let mut received = Vec::new();
    let mut records = Vec::new();
    loop {
        let HostMessage::File { operation, request } = raw.receive() else {
            panic!("the host sends a file request");
        };
        let FileRequest::Write {
            offset,
            create,
            shorten,
            bytes,
            ..
        } = request
        else {
            panic!("a whole-file write asks for writes");
        };
        assert_eq!(offset, u64::try_from(received.len()).expect("an offset"));
        received.extend_from_slice(&bytes);
        records.push((create, shorten, bytes.len()));
        raw.send(GuestMessage::file_outcome(
            operation,
            FileOutcome::Written {
                bytes: u64::try_from(bytes.len()).expect("a count"),
            },
        ));
        if shorten {
            break;
        }
    }

    let (_host, outcome) = host_thread.join().expect("host thread").expect("write");
    assert_eq!(outcome, WholeFileWrite::Written);
    assert_eq!(received, contents);
    assert_eq!(
        records,
        vec![(true, false, MAX_CHUNK_BYTES), (false, true, 17)]
    );
    assert_eq!(observed.poison(), 0);
}

fn body(length: usize) -> Vec<u8> {
    (0..length)
        .map(|index| u8::try_from(index % 251).expect("a byte"))
        .collect()
}

fn exists() -> FileRequest {
    FileRequest::Exists {
        path: b"/tmp/file".as_slice().into(),
    }
}

pub(super) fn repaired_host() -> (Repaired, RawGuest, Observation) {
    let (host, mut raw, observed) = connected_host();
    let host_thread = thread::spawn(move || host.prepare_and_probe());
    assert!(matches!(raw.receive(), HostMessage::PrepareAndProbe { .. }));
    raw.send(GuestMessage::repair_complete(operation(3)));
    raw.send(GuestMessage::terminal(
        operation(3),
        TerminalReport::new(TerminalStatus::Exited(0), 0, 0).expect("report"),
    ));
    let host = host_thread
        .join()
        .expect("host thread")
        .expect("repair and probe");
    (host, raw, observed)
}

fn connected_host() -> (Host, RawGuest, Observation) {
    let (host_material, guest_material) = launch();
    let (host_io, guest_io, host_observed, _guest_observed) = pair();
    let guest_thread = thread::spawn(move || RawGuest::connect(guest_material, guest_io));
    let host = HostControl::connect(host_material, host_io).expect("host connect");
    (
        host,
        guest_thread.join().expect("guest thread"),
        host_observed,
    )
}

pub(super) fn operation(value: u8) -> OperationId {
    OperationId::new([value; 16]).expect("operation")
}

pub(super) fn control_error<T>(result: Result<T, ControlError>) -> ControlError {
    match result {
        Ok(value) => {
            drop(value);
            panic!("operation unexpectedly succeeded");
        }
        Err(error) => error,
    }
}
