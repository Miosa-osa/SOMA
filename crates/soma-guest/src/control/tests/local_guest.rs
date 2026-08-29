use std::{thread, thread::JoinHandle};

use crate::{
    ControlError, ControlFailureClass, GuestCommand, GuestControl, GuestRequest, HostControl,
    OperationId, TerminalStatus,
};

use super::support::{MemoryIo, Observation, deadline, launch, pair};

type Guest = GuestControl<MemoryIo>;
type Host = HostControl<MemoryIo>;
type RepairedHost = super::super::RepairedHostControl<MemoryIo>;

#[test]
fn output_or_terminal_before_repair_poison_the_guest_once() {
    for illegal in [IllegalReport::Output, IllegalReport::Terminal] {
        let (host_thread, guest, host_observed, guest_observed) = awaiting_repair();
        let error = match illegal {
            IllegalReport::Output => control_error(guest.stdout(vec![1], deadline())),
            IllegalReport::Terminal => {
                control_error(guest.terminal(TerminalStatus::Exited(0), deadline()))
            }
        };
        assert_eq!(error.class(), ControlFailureClass::Lifecycle);
        assert_eq!(guest_observed.poison(), 1);
        control_error(host_thread.join().expect("host thread"));
        assert_eq!(host_observed.poison(), 1);
    }
}

#[test]
fn duplicate_local_repair_poisons_the_guest_once() {
    let (host_thread, guest, _host_observed, guest_observed) = awaiting_repair();
    let guest = guest
        .repair_complete(deadline())
        .expect("first repair report");

    let error = control_error(guest.repair_complete(deadline()));
    assert_eq!(error.class(), ControlFailureClass::Lifecycle);
    assert_eq!(guest_observed.poison(), 1);
    control_error(host_thread.join().expect("host thread"));
}

#[test]
fn local_output_allowance_plus_one_poisons_the_guest_once() {
    let (host, guest, _host_observed, guest_observed) = repaired_owners();
    let execute = operation(7);
    let host_thread = thread::spawn(move || host.execute(execute, command(3)));
    let (guest, request) = guest.next_request(deadline()).expect("execute request");
    assert!(matches!(request, GuestRequest::Execute { .. }));

    let error = control_error(guest.stdout(vec![1; 4], deadline()));
    assert_eq!(error.class(), ControlFailureClass::Accounting);
    assert_eq!(guest_observed.poison(), 1);
    control_error(host_thread.join().expect("host thread"));
}

#[test]
fn incorrect_local_output_limit_poisons_the_guest_once() {
    let (host, guest, _host_observed, guest_observed) = repaired_owners();
    let execute = operation(7);
    let host_thread = thread::spawn(move || host.execute(execute, command(5)));
    let (guest, _) = guest.next_request(deadline()).expect("execute request");
    let guest = guest
        .stdout(vec![1; 3], deadline())
        .expect("partial output");

    let error = control_error(guest.terminal(TerminalStatus::OutputLimit, deadline()));
    assert_eq!(error.class(), ControlFailureClass::Accounting);
    assert_eq!(guest_observed.poison(), 1);
    control_error(host_thread.join().expect("host thread"));
}

#[test]
fn receiving_while_an_operation_is_active_poisons_once() {
    let (host, guest, _host_observed, guest_observed) = repaired_owners();
    let execute = operation(7);
    let host_thread = thread::spawn(move || host.execute(execute, command(1)));
    let (guest, _) = guest.next_request(deadline()).expect("execute request");

    let error = control_error(guest.next_request(deadline()));
    assert_eq!(error.class(), ControlFailureClass::Lifecycle);
    assert_eq!(guest_observed.poison(), 1);
    control_error(host_thread.join().expect("host thread"));
}

fn awaiting_repair() -> (
    JoinHandle<Result<RepairedHost, crate::ControlError>>,
    Guest,
    Observation,
    Observation,
) {
    let (host, guest, host_observed, guest_observed) = connected_owners();
    let host_thread = thread::spawn(move || host.prepare_and_probe());
    let (guest, request) = guest.next_request(deadline()).expect("prepare request");
    assert!(matches!(request, GuestRequest::PrepareAndProbe { .. }));
    (host_thread, guest, host_observed, guest_observed)
}

fn repaired_owners() -> (RepairedHost, Guest, Observation, Observation) {
    let (host_thread, guest, host_observed, guest_observed) = awaiting_repair();
    let guest = guest.repair_complete(deadline()).expect("repair report");
    let guest = guest
        .terminal(TerminalStatus::Exited(0), deadline())
        .expect("probe terminal");
    let host = host_thread
        .join()
        .expect("host thread")
        .expect("ready host");
    (host, guest, host_observed, guest_observed)
}

fn connected_owners() -> (Host, Guest, Observation, Observation) {
    let (host_material, guest_material) = launch();
    let (host_io, guest_io, host_observed, guest_observed) = pair();
    let guest_thread = thread::spawn(move || {
        GuestControl::connect(guest_material, guest_io, deadline()).expect("guest connect")
    });
    let host = HostControl::connect(host_material, host_io).expect("host connect");
    (
        host,
        guest_thread.join().expect("guest thread"),
        host_observed,
        guest_observed,
    )
}

fn operation(value: u8) -> OperationId {
    OperationId::new([value; 16]).expect("operation")
}

fn command(output: u64) -> GuestCommand {
    GuestCommand::new(b"/bin/true".to_vec(), vec![], 10, output).expect("command")
}

enum IllegalReport {
    Output,
    Terminal,
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
