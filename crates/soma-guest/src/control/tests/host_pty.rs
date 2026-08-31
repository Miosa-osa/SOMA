//! The terminal exchange driven through both real control owners, and its refusals.

use std::thread;

use crate::{
    ControlError, ControlFailureClass, ControlStage, GuestControl, GuestMessage, GuestRequest,
    HostControl, HostMessage, OperationId, PtyFailure, PtyOutcome, PtyRequest, PtySize,
    TerminalReport, TerminalStatus,
};

use super::super::RepairedHostControl;
use super::support::{MemoryIo, Observation, RawGuest, deadline, launch, pair};

type Guest = GuestControl<MemoryIo>;
type Repaired = RepairedHostControl<MemoryIo>;

fn size(columns: u16, rows: u16) -> PtySize {
    PtySize::new(columns, rows).expect("bounded dimensions")
}

/// Every request the terminal protocol carries survives the round trip through both owners.
///
/// The guest side answers each one from a script rather than from a real pseudo-terminal,
/// because what is under test here is the exchange: the request arrives decoded, the single
/// answer returns the owner to idle, and the next request can then be sent on the same session.
#[test]
fn every_request_reaches_the_guest_and_its_answer_returns() {
    let exchanges = [
        (
            PtyRequest::Open(size(80, 24)),
            PtyOutcome::Opened(size(80, 24)),
        ),
        (
            PtyRequest::Write {
                bytes: b"echo hello\n".to_vec().into(),
            },
            PtyOutcome::Wrote { bytes: 11 },
        ),
        (
            PtyRequest::Read { wait_millis: 250 },
            PtyOutcome::Output {
                bytes: b"hello\r\n".to_vec().into(),
                end: false,
            },
        ),
        (
            PtyRequest::Resize(size(132, 43)),
            PtyOutcome::Resized(size(132, 43)),
        ),
        (
            PtyRequest::Read { wait_millis: 0 },
            PtyOutcome::Output {
                bytes: Box::default(),
                end: true,
            },
        ),
        (PtyRequest::Close, PtyOutcome::Closed),
    ];
    let (host, guest, host_observed, guest_observed) = repaired_owners();
    let sent: Vec<PtyRequest> = exchanges
        .iter()
        .map(|(request, _)| request.clone())
        .collect();
    let answers: Vec<PtyOutcome> = exchanges
        .iter()
        .map(|(_, outcome)| outcome.clone())
        .collect();
    let guest_thread = thread::spawn(move || answer_each(guest, &answers));
    let mut host = host;
    let mut received = Vec::new();
    for request in sent.clone() {
        let (next, outcome) = host.pty(request).expect("one terminal exchange");
        host = next;
        received.push(outcome);
    }

    let seen = guest_thread.join().expect("guest thread");
    assert_eq!(seen, sent, "the guest saw exactly what the host sent");
    assert_eq!(
        received,
        exchanges
            .iter()
            .map(|(_, outcome)| outcome.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(host_observed.poison(), 0);
    assert_eq!(guest_observed.poison(), 0);
}

/// A refusal is an answer, so it returns to the caller and leaves the session usable.
#[test]
fn a_refusal_is_delivered_rather_than_poisoning_the_session() {
    let (host, guest, host_observed, guest_observed) = repaired_owners();
    let answers = vec![
        PtyOutcome::Failed(PtyFailure::NoSession),
        PtyOutcome::Failed(PtyFailure::AlreadyOpen),
    ];
    let guest_thread = thread::spawn(move || answer_each(guest, &answers));

    let (host, first) = host.pty(PtyRequest::Close).expect("a refused close");
    let (host, second) = host
        .pty(PtyRequest::Open(size(80, 24)))
        .expect("a refused open");

    drop(host);
    guest_thread.join().expect("guest thread");
    assert_eq!(first, PtyOutcome::Failed(PtyFailure::NoSession));
    assert_eq!(second, PtyOutcome::Failed(PtyFailure::AlreadyOpen));
    assert_eq!(host_observed.poison(), 0);
    assert_eq!(guest_observed.poison(), 0);
}

/// An answer under an identity the host did not ask about could be a replay of an older one.
#[test]
fn an_answer_under_another_operation_is_rejected() {
    let (host, mut raw, observed) = repaired_host();
    let host_thread = thread::spawn(move || host.pty(PtyRequest::Close));
    let HostMessage::Pty { .. } = raw.receive() else {
        panic!("the host sends a terminal request");
    };
    raw.send(GuestMessage::pty_outcome(operation(9), PtyOutcome::Closed));

    let error = control_error(host_thread.join().expect("host thread"));
    assert_eq!(error.stage(), ControlStage::Pty);
    assert_eq!(error.class(), ControlFailureClass::Protocol);
    assert_eq!(observed.poison(), 1);
}

/// A terminal request is answered by a terminal outcome and by nothing else.
#[test]
fn an_answer_of_another_kind_is_rejected() {
    let (host, mut raw, observed) = repaired_host();
    let host_thread = thread::spawn(move || host.pty(PtyRequest::Close));
    let HostMessage::Pty { operation, .. } = raw.receive() else {
        panic!("the host sends a terminal request");
    };
    raw.send(GuestMessage::terminal(
        operation,
        TerminalReport::new(TerminalStatus::Exited(0), 0, 0).expect("report"),
    ));

    let error = control_error(host_thread.join().expect("host thread"));
    assert_eq!(error.stage(), ControlStage::Pty);
    assert_eq!(error.class(), ControlFailureClass::Lifecycle);
    assert_eq!(observed.poison(), 1);
}

/// Each request names itself, so a second one cannot be answered by the first one's reply.
#[test]
fn each_request_mints_its_own_identity() {
    let (host, mut raw, observed) = repaired_host();
    let host_thread = thread::spawn(move || {
        let (host, _) = host.pty(PtyRequest::Close).expect("first request");
        let (host, _) = host.pty(PtyRequest::Close).expect("second request");
        drop(host);
    });
    let mut seen = Vec::new();
    for _ in 0..2 {
        let HostMessage::Pty { operation, request } = raw.receive() else {
            panic!("the host sends a terminal request");
        };
        assert_eq!(request, PtyRequest::Close);
        seen.push(operation);
        raw.send(GuestMessage::pty_outcome(
            operation,
            PtyOutcome::Failed(PtyFailure::NoSession),
        ));
    }

    host_thread.join().expect("host thread");
    assert_ne!(seen[0], seen[1]);
    assert_eq!(observed.poison(), 0);
}

/// Answers one terminal request per scripted outcome and returns the requests it saw.
fn answer_each(guest: Guest, answers: &[PtyOutcome]) -> Vec<PtyRequest> {
    let mut guest = guest;
    let mut seen = Vec::new();
    for outcome in answers {
        let (next, request) = guest.next_request(deadline()).expect("a terminal request");
        let GuestRequest::Pty { request, .. } = request else {
            panic!("expected a terminal request, got {request:?}");
        };
        seen.push(request);
        guest = next.pty_outcome(outcome, deadline()).expect("one answer");
    }
    seen
}

fn repaired_owners() -> (Repaired, Guest, Observation, Observation) {
    let (host_material, guest_material) = launch();
    let (host_io, guest_io, host_observed, guest_observed) = pair();
    let guest_thread = thread::spawn(move || {
        let guest =
            GuestControl::connect(guest_material, guest_io, deadline()).expect("guest connect");
        let (guest, _) = guest.next_request(deadline()).expect("prepare request");
        let guest = guest.repair_complete(deadline()).expect("repair report");
        guest
            .terminal(TerminalStatus::Exited(0), deadline())
            .expect("probe terminal")
    });
    let host = HostControl::connect(host_material, host_io).expect("host connect");
    let host = host.prepare_and_probe().expect("ready host");
    (
        host,
        guest_thread.join().expect("guest thread"),
        host_observed,
        guest_observed,
    )
}

fn repaired_host() -> (Repaired, RawGuest, Observation) {
    let (host_material, guest_material) = launch();
    let (host_io, guest_io, host_observed, _guest_observed) = pair();
    let guest_thread = thread::spawn(move || RawGuest::connect(guest_material, guest_io));
    let host = HostControl::connect(host_material, host_io).expect("host connect");
    let mut raw = guest_thread.join().expect("guest thread");
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
    (host, raw, host_observed)
}

fn operation(value: u8) -> OperationId {
    OperationId::new([value; 16]).expect("operation")
}

fn control_error<T>(result: Result<T, ControlError>) -> ControlError {
    match result {
        Ok(value) => {
            drop(value);
            panic!("operation unexpectedly succeeded");
        }
        Err(error) => error,
    }
}
