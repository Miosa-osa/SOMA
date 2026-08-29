use std::thread;

use crate::{
    ControlError, ControlFailureClass, GuestCommand, GuestControl, GuestMessage, HostMessage,
    OperationId, TerminalStatus,
};

use super::support::{MemoryIo, Observation, RawHost, deadline, launch, pair};

#[test]
fn every_host_kind_illegal_before_repair_poisons_once() {
    let hostile = [
        HostMessage::prepare_and_probe(operation(9)),
        HostMessage::execute(operation(7), command()),
        HostMessage::shutdown(operation(7)),
    ];
    for message in hostile {
        let (guest, mut raw, observed) = connected_guest();
        raw.send_payload(&message.encode().expect("host message"));

        let error = control_error(guest.next_request(deadline()));
        assert!(matches!(
            error.class(),
            ControlFailureClass::Protocol | ControlFailureClass::Lifecycle
        ));
        assert_eq!(observed.poison(), 1);
    }
}

#[test]
fn authenticated_guest_direction_bytes_from_host_poison_once() {
    let (guest, mut raw, observed) = connected_guest();
    raw.send_payload(
        &GuestMessage::repair_complete(operation(3))
            .encode()
            .expect("wrong-direction message"),
    );

    let error = control_error(guest.next_request(deadline()));
    assert_eq!(error.class(), ControlFailureClass::Protocol);
    assert_eq!(observed.poison(), 1);
}

#[test]
fn a_second_prepare_after_ready_poisons_once() {
    let (guest, mut raw, observed) = connected_guest();
    raw.send_payload(
        &HostMessage::prepare_and_probe(operation(3))
            .encode()
            .expect("prepare"),
    );
    let (guest, _) = guest.next_request(deadline()).expect("prepare request");
    let guest = guest.repair_complete(deadline()).expect("repair report");
    assert!(matches!(raw.receive(), GuestMessage::RepairComplete { .. }));
    let guest = guest
        .terminal(TerminalStatus::Exited(0), deadline())
        .expect("probe terminal");
    assert!(matches!(raw.receive(), GuestMessage::Terminal { .. }));
    raw.send_payload(
        &HostMessage::prepare_and_probe(operation(3))
            .encode()
            .expect("late prepare"),
    );

    let error = control_error(guest.next_request(deadline()));
    assert_eq!(error.class(), ControlFailureClass::Lifecycle);
    assert_eq!(observed.poison(), 1);
}

#[test]
fn an_execute_operation_identity_cannot_be_reused() {
    let (guest, mut raw, observed) = connected_guest();
    raw.send_payload(
        &HostMessage::prepare_and_probe(operation(3))
            .encode()
            .expect("prepare"),
    );
    let (guest, _) = guest.next_request(deadline()).expect("prepare request");
    let guest = guest.repair_complete(deadline()).expect("repair report");
    assert!(matches!(raw.receive(), GuestMessage::RepairComplete { .. }));
    let guest = guest
        .terminal(TerminalStatus::Exited(0), deadline())
        .expect("probe terminal");
    assert!(matches!(raw.receive(), GuestMessage::Terminal { .. }));
    let execute = operation(7);
    raw.send_payload(
        &HostMessage::execute(execute, command())
            .encode()
            .expect("first execute"),
    );
    let (guest, _) = guest
        .next_request(deadline())
        .expect("first execute request");
    let guest = guest
        .terminal(TerminalStatus::Exited(0), deadline())
        .expect("execute terminal");
    assert!(matches!(raw.receive(), GuestMessage::Terminal { .. }));
    raw.send_payload(
        &HostMessage::execute(execute, command())
            .encode()
            .expect("replayed execute"),
    );

    let error = control_error(guest.next_request(deadline()));
    assert_eq!(error.class(), ControlFailureClass::Lifecycle);
    assert_eq!(observed.poison(), 1);
}

fn connected_guest() -> (GuestControl<MemoryIo>, RawHost, Observation) {
    let (host_material, guest_material, responder) = launch();
    let public = *responder.public_key();
    let (host_io, guest_io, _, guest_observed) = pair();
    let guest_thread = thread::spawn(move || {
        GuestControl::connect(
            guest_material,
            responder.private_key(),
            guest_io,
            deadline(),
        )
        .expect("guest connect")
    });
    let host = RawHost::connect(host_material, &public, host_io);
    (
        guest_thread.join().expect("guest thread"),
        host,
        guest_observed,
    )
}

fn command() -> GuestCommand {
    GuestCommand::new(b"/bin/true".to_vec(), vec![], 10, 1).expect("command")
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
