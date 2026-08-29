use std::{thread, time::Instant};

use crate::{
    ControlError, ControlFailureClass, ControlIo, GuestCommand, GuestControl, GuestMessage,
    HostControl, HostControlIo, HostMessage, OperationId, OutputChunk, TerminalReport,
    TerminalStatus,
};

use super::support::{MemoryIo, Observation, RawGuest, RawHost, deadline, launch, pair};

type Host = HostControl<MemoryIo>;
type Repaired = super::super::RepairedHostControl<MemoryIo>;

#[test]
fn every_guest_kind_illegal_before_repair_poisons_once() {
    for response in [
        GuestMessage::repair_complete(operation(9)),
        GuestMessage::stdout(operation(3), chunk(1)),
        GuestMessage::stderr(operation(3), chunk(1)),
        GuestMessage::terminal(operation(3), report(TerminalStatus::Exited(0), 0, 0)),
        GuestMessage::shutdown_ack(operation(3)),
    ] {
        let (host, mut raw, observed) = connected_host();
        let host_thread = thread::spawn(move || host.prepare_and_probe());
        assert!(matches!(raw.receive(), HostMessage::PrepareAndProbe { .. }));
        raw.send(response);
        let error = control_error(host_thread.join().expect("host thread"));
        assert!(matches!(
            error.class(),
            ControlFailureClass::Protocol
                | ControlFailureClass::Lifecycle
                | ControlFailureClass::Accounting
        ));
        assert_eq!(observed.repair(), 0);
        assert_eq!(observed.poison(), 1);
    }
}

#[test]
fn authenticated_host_direction_bytes_from_guest_poison_once() {
    let (host, mut raw, observed) = connected_host();
    let host_thread = thread::spawn(move || host.prepare_and_probe());
    assert!(matches!(raw.receive(), HostMessage::PrepareAndProbe { .. }));
    raw.send_payload(
        &HostMessage::shutdown(operation(3))
            .encode()
            .expect("wrong-direction message"),
    );

    let error = control_error(host_thread.join().expect("host thread"));
    assert_eq!(error.class(), ControlFailureClass::Protocol);
    assert_eq!(observed.poison(), 1);
}

#[test]
fn duplicate_repair_after_commit_poisons_once() {
    let (host, mut raw, observed) = connected_host();
    let host_thread = thread::spawn(move || host.prepare_and_probe());
    assert!(matches!(raw.receive(), HostMessage::PrepareAndProbe { .. }));
    raw.send(GuestMessage::repair_complete(operation(3)));
    raw.send(GuestMessage::repair_complete(operation(3)));

    control_error(host_thread.join().expect("host thread"));
    assert_eq!(observed.repair(), 1);
    assert_eq!(observed.poison(), 1);
}

#[test]
fn repair_commit_failure_poisons_once_before_probe_acceptance() {
    let (host_material, guest_material, responder) = launch();
    let public = *responder.public_key();
    let (host_io, guest_io, host_observed, _) = pair();
    let guest_thread =
        thread::spawn(move || RawGuest::connect(guest_material, responder.private_key(), guest_io));
    let host =
        HostControl::connect(host_material, &public, CommitFailIo(host_io)).expect("host connect");
    let mut raw = guest_thread.join().expect("guest thread");
    let host_thread = thread::spawn(move || host.prepare_and_probe());
    assert!(matches!(raw.receive(), HostMessage::PrepareAndProbe { .. }));
    raw.send(GuestMessage::repair_complete(operation(3)));

    let error = control_error(host_thread.join().expect("host thread"));
    assert_eq!(error.class(), ControlFailureClass::Io);
    assert_eq!(host_observed.repair(), 0);
    assert_eq!(host_observed.poison(), 1);
}

#[test]
fn fixed_probe_cannot_be_substituted_by_an_authenticated_host() {
    let (host_material, guest_material, responder) = launch();
    let public = *responder.public_key();
    let (host_io, guest_io, _host_observed, guest_observed) = pair();
    let guest_thread = thread::spawn(move || {
        GuestControl::connect(
            guest_material,
            responder.private_key(),
            guest_io,
            deadline(),
        )
        .expect("guest connect")
    });
    let mut raw_host = RawHost::connect(host_material, &public, host_io);
    let guest = guest_thread.join().expect("guest thread");
    let mut substituted = HostMessage::prepare_and_probe(operation(3))
        .encode()
        .expect("fixed probe");
    assert_eq!(&substituted[42..56], b"/proc/self/exe");
    substituted[42..56].copy_from_slice(b"/tmp/not-probe");
    raw_host.send_payload(&substituted);

    let error = control_error(guest.next_request(deadline()));
    assert_eq!(error.class(), ControlFailureClass::Protocol);
    assert_eq!(guest_observed.poison(), 1);
}

#[test]
fn execute_rejects_allowance_and_count_violations() {
    assert_execute_rejected(5, |raw, operation| {
        raw.send(GuestMessage::stdout(operation, chunk(6)));
    });
    assert_execute_rejected(5, |raw, operation| {
        raw.send(GuestMessage::stdout(operation, chunk(3)));
        raw.send(GuestMessage::terminal(
            operation,
            report(TerminalStatus::Exited(0), 2, 0),
        ));
    });
    assert_execute_rejected(5, |raw, operation| {
        raw.send(GuestMessage::stdout(operation, chunk(3)));
        raw.send(GuestMessage::terminal(
            operation,
            report(TerminalStatus::OutputLimit, 3, 0),
        ));
    });
}

#[test]
fn duplicate_terminal_is_rejected_by_the_next_exchange() {
    let (host, raw, observed) = connected_host();
    let (host, mut raw) = repaired(host, raw);
    let first = operation(7);
    let host_thread =
        thread::spawn(move || host.execute(first, command(1)).expect("first execute").0);
    assert!(matches!(raw.receive(), HostMessage::Execute { .. }));
    let terminal = GuestMessage::terminal(first, report(TerminalStatus::Exited(0), 0, 0));
    raw.send(terminal.clone());
    let host = host_thread.join().expect("host thread");
    raw.send(terminal);

    let error = control_error(host.execute(operation(8), command(1)));
    assert_eq!(error.class(), ControlFailureClass::Protocol);
    assert_eq!(observed.poison(), 1);
}

#[test]
fn operation_identity_cannot_be_reused_to_accept_a_late_terminal() {
    let (host, raw, observed) = connected_host();
    let (host, mut raw) = repaired(host, raw);
    let first = operation(7);
    let host_thread =
        thread::spawn(move || host.execute(first, command(1)).expect("first execute").0);
    assert!(matches!(raw.receive(), HostMessage::Execute { .. }));
    let terminal = GuestMessage::terminal(first, report(TerminalStatus::Exited(0), 0, 0));
    raw.send(terminal.clone());
    let host = host_thread.join().expect("host thread");
    raw.send(terminal);

    let error = control_error(host.execute(first, command(1)));
    assert_eq!(error.class(), ControlFailureClass::Lifecycle);
    assert_eq!(observed.poison(), 1);
}

#[test]
fn output_after_terminal_is_rejected_by_the_next_exchange() {
    let (host, raw, observed) = connected_host();
    let (host, mut raw) = repaired(host, raw);
    let first = operation(7);
    let host_thread =
        thread::spawn(move || host.execute(first, command(1)).expect("first execute").0);
    assert!(matches!(raw.receive(), HostMessage::Execute { .. }));
    raw.send(GuestMessage::terminal(
        first,
        report(TerminalStatus::Exited(0), 0, 0),
    ));
    let host = host_thread.join().expect("host thread");
    raw.send(GuestMessage::stdout(first, chunk(1)));

    control_error(host.execute(operation(8), command(1)));
    assert_eq!(observed.poison(), 1);
}

#[test]
fn shutdown_requires_the_exact_operation_acknowledgement() {
    let (host, raw, observed) = connected_host();
    let (host, mut raw) = repaired(host, raw);
    let host_thread = thread::spawn(move || host.shutdown(operation(7)));
    assert!(matches!(raw.receive(), HostMessage::Shutdown { .. }));
    raw.send(GuestMessage::shutdown_ack(operation(8)));

    let error = control_error(host_thread.join().expect("host thread"));
    assert_eq!(error.class(), ControlFailureClass::Protocol);
    assert_eq!(observed.poison(), 1);
}

fn assert_execute_rejected(allowance: u64, respond: impl FnOnce(&mut RawGuest, OperationId)) {
    let (host, raw, observed) = connected_host();
    let (host, mut raw) = repaired(host, raw);
    let execute = operation(7);
    let host_thread = thread::spawn(move || host.execute(execute, command(allowance)));
    assert!(matches!(raw.receive(), HostMessage::Execute { .. }));
    respond(&mut raw, execute);
    let error = control_error(host_thread.join().expect("host thread"));
    assert_eq!(error.class(), ControlFailureClass::Accounting);
    assert_eq!(observed.poison(), 1);
}

fn connected_host() -> (Host, RawGuest, Observation) {
    let (host_material, guest_material, responder) = launch();
    let public = *responder.public_key();
    let (host_io, guest_io, host_observed, _guest_observed) = pair();
    let guest_thread =
        thread::spawn(move || RawGuest::connect(guest_material, responder.private_key(), guest_io));
    let host = HostControl::connect(host_material, &public, host_io).expect("host connect");
    (
        host,
        guest_thread.join().expect("guest thread"),
        host_observed,
    )
}

fn repaired(host: Host, mut raw: RawGuest) -> (Repaired, RawGuest) {
    let host_thread = thread::spawn(move || host.prepare_and_probe());
    assert!(matches!(raw.receive(), HostMessage::PrepareAndProbe { .. }));
    raw.send(GuestMessage::repair_complete(operation(3)));
    raw.send(GuestMessage::terminal(
        operation(3),
        report(TerminalStatus::Exited(0), 0, 0),
    ));
    (
        host_thread
            .join()
            .expect("host thread")
            .expect("repair and probe"),
        raw,
    )
}

fn operation(value: u8) -> OperationId {
    OperationId::new([value; 16]).expect("operation")
}

fn command(output: u64) -> GuestCommand {
    GuestCommand::new(b"/bin/true".to_vec(), vec![], 10, output).expect("command")
}

fn chunk(length: usize) -> OutputChunk {
    OutputChunk::new(vec![0xA5; length]).expect("chunk")
}

fn report(status: TerminalStatus, stdout: u32, stderr: u32) -> TerminalReport {
    TerminalReport::new(status, stdout, stderr).expect("report")
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

struct CommitFailIo(MemoryIo);

impl ControlIo for CommitFailIo {
    type Error = ();

    fn read_exact(&mut self, bytes: &mut [u8], deadline: Instant) -> Result<(), Self::Error> {
        self.0.read_exact(bytes, deadline)
    }

    fn write_all(&mut self, bytes: &[u8], deadline: Instant) -> Result<(), Self::Error> {
        self.0.write_all(bytes, deadline)
    }

    fn poison(&mut self) {
        self.0.poison();
    }
}

impl HostControlIo for CommitFailIo {
    fn commit_repair(&mut self, _deadline: Instant) -> Result<(), Self::Error> {
        Err(())
    }
}
