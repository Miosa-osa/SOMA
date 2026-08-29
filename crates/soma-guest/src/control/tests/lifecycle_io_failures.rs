use std::thread;

use crate::{
    ControlError, ControlIo, GuestCommand, GuestControl, HostControl, OperationId, TerminalStatus,
};

use super::{
    io_failures::{successful_guest_repair_traffic, successful_host_repair_traffic},
    support::{
        deadline,
        fault::{Direction, FaultIo},
        launch, pair,
    },
};

#[test]
fn host_execute_and_shutdown_io_failures_cover_every_byte() {
    let repair = successful_host_repair_traffic();
    let complete = successful_host_lifecycle_traffic();

    for fail_at in repair.0..complete.0 {
        assert_host_lifecycle_fails(Direction::Read, fail_at);
    }
    for fail_at in repair.1..complete.1 {
        assert_host_lifecycle_fails(Direction::Write, fail_at);
    }
}

#[test]
fn guest_execute_and_shutdown_io_failures_cover_every_byte() {
    let repair = successful_guest_repair_traffic();
    let complete = successful_guest_lifecycle_traffic();

    for fail_at in repair.0..complete.0 {
        assert_guest_lifecycle_fails(Direction::Read, fail_at);
    }
    for fail_at in repair.1..complete.1 {
        assert_guest_lifecycle_fails(Direction::Write, fail_at);
    }
}

fn successful_host_lifecycle_traffic() -> (usize, usize) {
    let (host_material, guest_material, responder) = launch();
    let public = *responder.public_key();
    let (host_io, guest_io, _, _) = pair();
    let (host_io, traffic) = FaultIo::new(host_io, Direction::Read, None);
    let guest_thread = thread::spawn(move || {
        let guest = GuestControl::connect(
            guest_material,
            responder.private_key(),
            guest_io,
            deadline(),
        )
        .expect("guest connect");
        drive_guest(guest).expect("guest lifecycle");
    });
    let host = HostControl::connect(host_material, &public, host_io)
        .expect("host connect")
        .prepare_and_probe()
        .expect("host Ready");
    drive_host(host).expect("host lifecycle");
    guest_thread.join().expect("guest thread");
    (traffic.read(), traffic.written())
}

fn successful_guest_lifecycle_traffic() -> (usize, usize) {
    let (host_material, guest_material, responder) = launch();
    let public = *responder.public_key();
    let (host_io, guest_io, _, _) = pair();
    let (guest_io, traffic) = FaultIo::new(guest_io, Direction::Write, None);
    let host_thread = thread::spawn(move || {
        let host = HostControl::connect(host_material, &public, host_io)
            .expect("host connect")
            .prepare_and_probe()
            .expect("host Ready");
        drive_host(host).expect("host lifecycle");
    });
    let guest = GuestControl::connect(
        guest_material,
        responder.private_key(),
        guest_io,
        deadline(),
    )
    .expect("guest connect");
    drive_guest(guest).expect("guest lifecycle");
    host_thread.join().expect("host thread");
    (traffic.read(), traffic.written())
}

fn assert_host_lifecycle_fails(direction: Direction, fail_at: usize) {
    let (host_material, guest_material, responder) = launch();
    let public = *responder.public_key();
    let (host_io, guest_io, host_observed, _) = pair();
    let (host_io, _) = FaultIo::new(host_io, direction, Some(fail_at));
    let guest_thread = thread::spawn(move || {
        let guest = GuestControl::connect(
            guest_material,
            responder.private_key(),
            guest_io,
            deadline(),
        )?;
        drive_guest(guest)
    });
    let host = HostControl::connect(host_material, &public, host_io)
        .expect("host connect")
        .prepare_and_probe()
        .expect("host Ready");

    assert!(drive_host(host).is_err());
    assert_eq!(host_observed.poison(), 1);
    let _ = guest_thread.join().expect("guest thread");
}

fn assert_guest_lifecycle_fails(direction: Direction, fail_at: usize) {
    let (host_material, guest_material, responder) = launch();
    let public = *responder.public_key();
    let (host_io, guest_io, _, guest_observed) = pair();
    let (guest_io, _) = FaultIo::new(guest_io, direction, Some(fail_at));
    let host_thread = thread::spawn(move || {
        let host = HostControl::connect(host_material, &public, host_io)?;
        let host = host.prepare_and_probe()?;
        drive_host(host)
    });
    let guest = GuestControl::connect(
        guest_material,
        responder.private_key(),
        guest_io,
        deadline(),
    )
    .expect("guest connect");

    assert!(drive_guest(guest).is_err());
    assert_eq!(guest_observed.poison(), 1);
    let _ = host_thread.join().expect("host thread");
}

fn drive_host<I: crate::HostControlIo>(
    host: super::super::RepairedHostControl<I>,
) -> Result<(), ControlError> {
    let (host, outcome) = host.execute(operation(7), command())?;
    assert!(
        outcome.status() == TerminalStatus::Exited(17) && outcome.stdout() == b"ok",
        "unexpected authenticated outcome"
    );
    host.shutdown(operation(8))
}

fn drive_guest<I: ControlIo>(guest: GuestControl<I>) -> Result<(), ControlError> {
    let (guest, _) = guest.next_request(deadline())?;
    let guest = guest.repair_complete(deadline())?;
    let guest = guest.terminal(TerminalStatus::Exited(0), deadline())?;
    let (guest, _) = guest.next_request(deadline())?;
    let guest = guest.stdout(b"ok".to_vec(), deadline())?;
    let guest = guest.terminal(TerminalStatus::Exited(17), deadline())?;
    let (guest, _) = guest.next_request(deadline())?;
    guest.shutdown_ack(deadline())
}

fn operation(value: u8) -> OperationId {
    OperationId::new([value; 16]).expect("operation")
}

fn command() -> GuestCommand {
    GuestCommand::new(b"/bin/true".to_vec(), vec![], 10, 2).expect("command")
}
