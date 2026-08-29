use std::thread;

use crate::{ControlError, ControlIo, GuestControl, HostControl, TerminalStatus};

use super::support::{
    deadline,
    fault::{Direction, FaultIo},
    launch, pair,
};

#[test]
fn host_handshake_io_failure_at_every_byte_poisons_once() {
    let (read_bytes, write_bytes) = successful_host_traffic();
    assert!(read_bytes > 0);
    assert!(write_bytes > 0);

    for fail_at in 0..read_bytes {
        assert_host_connect_fails(Direction::Read, fail_at);
    }
    for fail_at in 0..write_bytes {
        assert_host_connect_fails(Direction::Write, fail_at);
    }
}

#[test]
fn responder_handshake_io_failure_at_every_byte_never_returns_an_owner() {
    let (read_bytes, write_bytes) = successful_guest_traffic();
    assert!(read_bytes > 0);
    assert!(write_bytes > 0);

    for fail_at in 0..read_bytes {
        assert_guest_connect_fails(Direction::Read, fail_at);
    }
    for fail_at in 0..write_bytes {
        assert_guest_connect_fails(Direction::Write, fail_at);
    }
}

#[test]
fn host_repair_exchange_io_failure_at_every_byte_poisons_once() {
    let handshake = successful_host_traffic();
    let complete = successful_host_repair_traffic();

    for fail_at in handshake.0..complete.0 {
        assert_host_repair_fails(Direction::Read, fail_at);
    }
    for fail_at in handshake.1..complete.1 {
        assert_host_repair_fails(Direction::Write, fail_at);
    }
}

#[test]
fn guest_repair_exchange_io_failure_at_every_byte_poisons_once() {
    let handshake = successful_guest_traffic();
    let complete = successful_guest_repair_traffic();

    for fail_at in handshake.0..complete.0 {
        assert_guest_repair_fails(Direction::Read, fail_at);
    }
    for fail_at in handshake.1..complete.1 {
        assert_guest_repair_fails(Direction::Write, fail_at);
    }
}

fn successful_host_traffic() -> (usize, usize) {
    let (host_material, guest_material, responder) = launch();
    let public = *responder.public_key();
    let (host_io, guest_io, _, _) = pair();
    let (host_io, traffic) = FaultIo::new(host_io, Direction::Read, None);
    let guest_thread = thread::spawn(move || {
        GuestControl::connect(
            guest_material,
            responder.private_key(),
            guest_io,
            deadline(),
        )
        .expect("guest owner")
    });
    let host = HostControl::connect(host_material, &public, host_io).expect("host owner");
    drop((host, guest_thread.join().expect("guest thread")));
    (traffic.read(), traffic.written())
}

fn successful_guest_traffic() -> (usize, usize) {
    let (host_material, guest_material, responder) = launch();
    let public = *responder.public_key();
    let (host_io, guest_io, _, _) = pair();
    let (guest_io, traffic) = FaultIo::new(guest_io, Direction::Write, None);
    let host_thread = thread::spawn(move || {
        HostControl::connect(host_material, &public, host_io).expect("host owner")
    });
    let guest = GuestControl::connect(
        guest_material,
        responder.private_key(),
        guest_io,
        deadline(),
    )
    .expect("guest owner");
    drop((guest, host_thread.join().expect("host thread")));
    (traffic.read(), traffic.written())
}

pub(super) fn successful_host_repair_traffic() -> (usize, usize) {
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
        .expect("guest owner");
        drive_guest_ready(guest).expect("guest Ready")
    });
    let host = HostControl::connect(host_material, &public, host_io)
        .expect("host owner")
        .prepare_and_probe()
        .expect("host Ready");
    drop((host, guest_thread.join().expect("guest thread")));
    (traffic.read(), traffic.written())
}

pub(super) fn successful_guest_repair_traffic() -> (usize, usize) {
    let (host_material, guest_material, responder) = launch();
    let public = *responder.public_key();
    let (host_io, guest_io, _, _) = pair();
    let (guest_io, traffic) = FaultIo::new(guest_io, Direction::Write, None);
    let host_thread = thread::spawn(move || {
        HostControl::connect(host_material, &public, host_io)
            .expect("host owner")
            .prepare_and_probe()
            .expect("host Ready")
    });
    let guest = GuestControl::connect(
        guest_material,
        responder.private_key(),
        guest_io,
        deadline(),
    )
    .expect("guest owner");
    let guest = drive_guest_ready(guest).expect("guest Ready");
    drop((guest, host_thread.join().expect("host thread")));
    (traffic.read(), traffic.written())
}

fn assert_host_connect_fails(direction: Direction, fail_at: usize) {
    let (host_material, guest_material, responder) = launch();
    let public = *responder.public_key();
    let (host_io, guest_io, host_observed, _) = pair();
    let (host_io, _) = FaultIo::new(host_io, direction, Some(fail_at));
    let guest_thread = thread::spawn(move || {
        GuestControl::connect(
            guest_material,
            responder.private_key(),
            guest_io,
            deadline(),
        )
    });
    let result = HostControl::connect(host_material, &public, host_io);

    assert_error(result);
    assert_eq!(host_observed.poison(), 1);
    let _ = guest_thread.join().expect("guest thread");
}

fn assert_guest_connect_fails(direction: Direction, fail_at: usize) {
    let (host_material, guest_material, responder) = launch();
    let public = *responder.public_key();
    let (host_io, guest_io, _, guest_observed) = pair();
    let (guest_io, _) = FaultIo::new(guest_io, direction, Some(fail_at));
    let host_thread = thread::spawn(move || HostControl::connect(host_material, &public, host_io));
    let result = GuestControl::connect(
        guest_material,
        responder.private_key(),
        guest_io,
        deadline(),
    );

    assert_error(result);
    assert_eq!(guest_observed.poison(), 1);
    let _ = host_thread.join().expect("host thread");
}

fn assert_host_repair_fails(direction: Direction, fail_at: usize) {
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
        drive_guest_ready(guest)
    });
    let host = HostControl::connect(host_material, &public, host_io).expect("handshake succeeds");

    assert_error(host.prepare_and_probe());
    assert_eq!(host_observed.poison(), 1);
    let _ = guest_thread.join().expect("guest thread");
}

fn assert_guest_repair_fails(direction: Direction, fail_at: usize) {
    let (host_material, guest_material, responder) = launch();
    let public = *responder.public_key();
    let (host_io, guest_io, _, guest_observed) = pair();
    let (guest_io, _) = FaultIo::new(guest_io, direction, Some(fail_at));
    let host_thread = thread::spawn(move || {
        let host = HostControl::connect(host_material, &public, host_io)?;
        host.prepare_and_probe()
    });
    let guest = GuestControl::connect(
        guest_material,
        responder.private_key(),
        guest_io,
        deadline(),
    )
    .expect("handshake succeeds");

    assert_error(drive_guest_ready(guest));
    assert_eq!(guest_observed.poison(), 1);
    let _ = host_thread.join().expect("host thread");
}

fn drive_guest_ready<I: ControlIo>(
    guest: GuestControl<I>,
) -> Result<GuestControl<I>, ControlError> {
    let (guest, _) = guest.next_request(deadline())?;
    let guest = guest.repair_complete(deadline())?;
    guest.terminal(TerminalStatus::Exited(0), deadline())
}

fn assert_error<T>(result: Result<T, ControlError>) {
    if let Ok(owner) = result {
        drop(owner);
        panic!("owner must not be returned after I/O failure");
    }
}
