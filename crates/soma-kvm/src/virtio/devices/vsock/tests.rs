//! End-to-end vsock tests: a model guest connects, exchanges bytes with the
//! host endpoint, and tears down, all through the transport.

use super::credit::HOST_BUF_ALLOC;
use super::guest_driver::{GUEST_PORT, GuestVsock, connect, hdr};
use super::packet::*;
use super::*;
use crate::virtio::devices::harness::Seg;

#[test]
fn guest_connects_exchanges_bytes_and_shuts_down() {
    let mut guest = GuestVsock::boot();
    connect(&mut guest);
    let endpoint = guest.device().endpoint().expect("endpoint");
    assert!(endpoint.is_open());
    assert_eq!(endpoint.peer_port(), GUEST_PORT);
    assert_eq!(
        endpoint.generation(),
        2,
        "driver reset bumped it before connect"
    );

    guest.send(hdr(VSOCK_OP_RW, 5, 0, 0), b"hello");
    let mut buf = [0u8; 16];
    let endpoint = guest.device().endpoint().expect("endpoint");
    assert_eq!(endpoint.read(&mut buf), 5);
    assert_eq!(&buf[..5], b"hello");
    assert_eq!(endpoint.write(b"world"), 5);
    guest.post_rx(4096);
    let packets = guest.recv();
    assert_eq!(packets.len(), 1, "{packets:?}");
    assert_eq!(packets[0].0.op, VSOCK_OP_RW);
    assert_eq!(packets[0].0.len, 5);
    assert_eq!(
        packets[0].0.fwd_cnt, 5,
        "consumed bytes are acknowledged inline"
    );
    assert_eq!(packets[0].1, b"world");

    guest.post_rx(4096);
    guest.send(
        hdr(
            VSOCK_OP_SHUTDOWN,
            0,
            VSOCK_SHUTDOWN_RCV | VSOCK_SHUTDOWN_SEND,
            5,
        ),
        &[],
    );
    let packets = guest.recv();
    assert_eq!(packets.len(), 1, "{packets:?}");
    assert_eq!(packets[0].0.op, VSOCK_OP_RST);
    let endpoint = guest.device().endpoint().expect("endpoint");
    assert!(!endpoint.is_open());
    assert!(endpoint.at_eof());
    assert_eq!(endpoint.write(b"late"), 0);
    guest.device().close_endpoint();
    assert!(guest.device().endpoint().is_none());
    assert_eq!(guest.device().counters().accepted, 1);
    assert_eq!(guest.device().counters().rst_sent, 1);
}

#[test]
fn host_shutdown_drains_data_then_guest_rst_closes() {
    let mut guest = GuestVsock::boot();
    connect(&mut guest);
    let endpoint = guest.device().endpoint().expect("endpoint");
    assert_eq!(endpoint.write(b"bye"), 3);
    endpoint.shutdown();
    assert_eq!(endpoint.write(b"more"), 0, "no writes after shutdown");
    guest.post_rx(4096);
    guest.post_rx(4096);
    let packets = guest.recv();
    let ops: Vec<u16> = packets.iter().map(|(header, _)| header.op).collect();
    assert_eq!(ops, vec![VSOCK_OP_RW, VSOCK_OP_SHUTDOWN]);
    assert_eq!(packets[1].0.flags, VSOCK_SHUTDOWN_RCV | VSOCK_SHUTDOWN_SEND);
    guest.send(hdr(VSOCK_OP_RST, 0, 0, 0), &[]);
    assert!(!guest.device().endpoint().expect("endpoint").is_open());
    guest.post_rx(4096);
    assert!(guest.recv().is_empty(), "no reply to a guest RST");
}

#[test]
fn credit_bounds_host_sends_and_credit_request_is_answered() {
    let mut guest = GuestVsock::boot();
    guest.post_rx(4096);
    let mut request = hdr(VSOCK_OP_REQUEST, 0, 0, 0);
    request.buf_alloc = 10;
    guest.send(request, &[]);
    assert_eq!(guest.recv()[0].0.op, VSOCK_OP_RESPONSE);
    let endpoint = guest.device().endpoint().expect("endpoint");
    assert_eq!(endpoint.write(&[7u8; 25]), 25);
    guest.post_rx(4096);
    guest.post_rx(4096);
    let packets = guest.recv();
    assert_eq!(packets.len(), 1, "only ten bytes of credit");
    assert_eq!(packets[0].1.len(), 10);
    assert_eq!(
        guest.device().endpoint().expect("endpoint").pending_write(),
        15
    );
    let mut update = hdr(VSOCK_OP_CREDIT_UPDATE, 0, 0, 0);
    update.buf_alloc = 10;
    update.fwd_cnt = 10;
    guest.send(update, &[]);
    let packets = guest.recv();
    assert_eq!(packets.len(), 1, "{packets:?}");
    assert_eq!(packets[0].1.len(), 10);
    let mut request = hdr(VSOCK_OP_CREDIT_REQUEST, 0, 0, 0);
    request.buf_alloc = 10;
    request.fwd_cnt = 20;
    guest.send(request, &[]);
    guest.post_rx(4096);
    guest.post_rx(4096);
    let packets = guest.recv();
    let ops: Vec<u16> = packets.iter().map(|(header, _)| header.op).collect();
    assert_eq!(
        ops,
        vec![VSOCK_OP_CREDIT_UPDATE, VSOCK_OP_RW],
        "control first"
    );
    assert_eq!(packets[1].1.len(), 5);
    assert_eq!(packets[0].0.buf_alloc, HOST_BUF_ALLOC);
}

#[test]
fn impossible_credit_and_window_overrun_reset_the_connection() {
    let mut guest = GuestVsock::boot();
    connect(&mut guest);
    guest.send(hdr(VSOCK_OP_RW, 0, 0, 0), &[]);
    let mut lying = hdr(VSOCK_OP_CREDIT_UPDATE, 0, 0, 0);
    lying.fwd_cnt = 1;
    guest.send(lying, &[]);
    guest.post_rx(4096);
    assert_eq!(guest.recv()[0].0.op, VSOCK_OP_RST);
    assert!(!guest.device().endpoint().expect("endpoint").is_open());

    let mut guest = GuestVsock::boot();
    connect(&mut guest);
    let chunk = vec![1u8; usize::try_from(MAX_PAYLOAD_LEN).expect("small")];
    guest.send(hdr(VSOCK_OP_RW, MAX_PAYLOAD_LEN, 0, 0), &chunk);
    assert_eq!(
        guest.device().endpoint().expect("endpoint").pending_read(),
        65536
    );
    guest.send(hdr(VSOCK_OP_RW, 1, 0, 0), &[2]);
    guest.post_rx(4096);
    assert_eq!(guest.recv()[0].0.op, VSOCK_OP_RST);
    let endpoint = guest.device().endpoint().expect("endpoint");
    assert!(!endpoint.is_open());
    assert_eq!(
        endpoint.pending_read(),
        65536,
        "accepted bytes stay readable"
    );
    assert_eq!(guest.device().counters().rejected, 1);
}

#[test]
fn packets_for_unknown_ports_or_stale_connections_get_rst_or_are_dropped() {
    let mut guest = GuestVsock::boot();
    guest.post_rx(4096);
    guest.post_rx(4096);
    let mut wrong_port = hdr(VSOCK_OP_REQUEST, 0, 0, 0);
    wrong_port.dst_port = 80;
    guest.send(wrong_port, &[]);
    let mut wrong_port_rw = hdr(VSOCK_OP_RW, 0, 0, 0);
    wrong_port_rw.dst_port = 80;
    guest.send(wrong_port_rw, &[]);
    let packets = guest.recv();
    assert_eq!(packets.len(), 1, "REQUEST gets RST, RW is dropped");
    assert_eq!(
        (packets[0].0.op, packets[0].0.dst_port),
        (VSOCK_OP_RST, GUEST_PORT)
    );

    guest.send(hdr(VSOCK_OP_RW, 0, 0, 0), &[]);
    guest.send(hdr(VSOCK_OP_RST, 0, 0, 0), &[]);
    let packets = guest.recv();
    assert_eq!(
        packets.len(),
        1,
        "RW without a connection gets RST, RST does not"
    );
    assert_eq!(packets[0].0.op, VSOCK_OP_RST);

    connect(&mut guest);
    guest.post_rx(4096);
    guest.send(hdr(VSOCK_OP_REQUEST, 0, 0, 0), &[]);
    assert_eq!(
        guest.recv()[0].0.op,
        VSOCK_OP_RST,
        "second connect is refused"
    );
    assert!(guest.device().endpoint().expect("endpoint").is_open());
    guest.post_rx(4096);
    let mut other = hdr(VSOCK_OP_RW, 0, 0, 0);
    other.src_port = GUEST_PORT + 1;
    guest.send(other, &[]);
    let packets = guest.recv();
    assert_eq!(
        (packets[0].0.op, packets[0].0.dst_port),
        (VSOCK_OP_RST, GUEST_PORT + 1)
    );
    assert!(guest.device().endpoint().expect("endpoint").is_open());
    assert_eq!(guest.device().counters().rejected, 6);
}

#[test]
fn receive_chains_are_sized_and_bad_shapes_are_returned_empty() {
    let mut guest = GuestVsock::boot();
    connect(&mut guest);
    guest
        .device()
        .endpoint()
        .expect("endpoint")
        .write(&[9u8; 100]);
    guest.post_rx(44 + 30);
    guest.post_rx(20);
    let readable = guest.rig.alloc_zeroed(4096);
    guest.post_rx_chain(&[Seg::readable(readable, 4096)], readable);
    guest.post_rx(4096);
    let packets = guest.recv();
    assert_eq!(packets.len(), 2);
    assert_eq!(packets[0].1.len(), 30, "payload fits the small chain");
    assert_eq!(packets[1].1.len(), 70);
    assert_eq!(guest.empty_returns, 2);
    assert_eq!(guest.device().counters().rx_dropped, 2);
    assert!(guest.t.is_active());
}
