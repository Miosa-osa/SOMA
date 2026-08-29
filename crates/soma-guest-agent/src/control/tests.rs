use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::thread;
use std::time::{Duration, Instant};

use soma_guest::ControlIo;

use super::{IoFault, StreamIo};

fn pair() -> (StreamIo<UnixStream>, UnixStream) {
    let (local, peer) = UnixStream::pair().expect("socket pair");
    (StreamIo::new(local), peer)
}

#[test]
fn complete_reads_and_writes_succeed_before_the_deadline() {
    let (mut io, mut peer) = pair();
    let deadline = Instant::now() + Duration::from_secs(5);

    io.write_all(b"hello", deadline).expect("write");
    let mut received = [0; 5];
    peer.read_exact(&mut received).expect("peer read");
    assert_eq!(&received, b"hello");

    peer.write_all(b"world").expect("peer write");
    let mut buffer = [0; 5];
    io.read_exact(&mut buffer, deadline).expect("read");
    assert_eq!(&buffer, b"world");
}

#[test]
fn an_already_elapsed_deadline_fails_without_touching_the_socket() {
    let (mut io, mut peer) = pair();
    let deadline = Instant::now()
        .checked_sub(Duration::from_millis(1))
        .expect("recent past");
    peer.write_all(b"late").expect("peer write");

    let mut buffer = [0; 4];
    assert_eq!(io.read_exact(&mut buffer, deadline), Err(IoFault::Expired));
    assert_eq!(io.write_all(b"x", deadline), Err(IoFault::Expired));
    assert_eq!(buffer, [0; 4]);
}

#[test]
fn a_silent_peer_expires_the_read_at_the_deadline() {
    let (mut io, _peer) = pair();
    let started = Instant::now();
    let deadline = started + Duration::from_millis(50);

    let mut buffer = [0; 1];
    assert_eq!(io.read_exact(&mut buffer, deadline), Err(IoFault::Expired));
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(40),
        "returned too early: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "returned too late: {elapsed:?}"
    );
}

#[test]
fn a_partial_frame_does_not_renew_the_budget() {
    let (mut io, mut peer) = pair();
    let deadline = Instant::now() + Duration::from_millis(60);
    let writer = thread::spawn(move || {
        peer.write_all(b"ab").expect("first half");
        thread::sleep(Duration::from_millis(200));
        let _ = peer.write_all(b"cd");
        peer
    });

    let mut buffer = [0; 4];
    assert_eq!(io.read_exact(&mut buffer, deadline), Err(IoFault::Expired));
    drop(writer.join().expect("writer"));
}

#[test]
fn a_closed_peer_is_an_io_failure() {
    let (mut io, peer) = pair();
    drop(peer);
    let deadline = Instant::now() + Duration::from_secs(1);

    let mut buffer = [0; 1];
    assert_eq!(io.read_exact(&mut buffer, deadline), Err(IoFault::Io));
}

#[test]
fn poison_closes_the_transport_locally_and_permanently() {
    let (mut io, mut peer) = pair();
    let deadline = Instant::now() + Duration::from_secs(1);

    io.poison();

    let mut buffer = [0; 1];
    assert_eq!(peer.read(&mut buffer).expect("peer sees end of stream"), 0);
    assert_eq!(io.read_exact(&mut buffer, deadline), Err(IoFault::Poisoned));
    assert_eq!(io.write_all(b"x", deadline), Err(IoFault::Poisoned));
}

#[test]
fn the_control_port_is_the_fixed_machine_contract_value() {
    assert_eq!(super::CONTROL_VSOCK_PORT, 0x534f_4d41);
    assert_eq!(libc::VMADDR_CID_HOST, 2);
}
