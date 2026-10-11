//! Streams beside the control connection: arming, isolation, rotation, and teardown.

use super::guest_driver::{CID, GUEST_BUF, GUEST_PORT, GuestVsock, connect, hdr};
use super::packet::*;
use super::streams::{MAX_STREAMS, SOMA_STREAM_PORT_BASE};
use super::*;

const STREAM_GUEST_PORT: u32 = 41_000;

/// A packet from the guest's stream-side port to a host stream port.
fn to_stream(host_port: u32, guest_port: u32, op: u16, len: u32, flags: u32) -> VsockHeader {
    VsockHeader {
        src_cid: CID,
        dst_cid: HOST_CID,
        src_port: guest_port,
        dst_port: host_port,
        len,
        ty: VSOCK_TYPE_STREAM,
        op,
        flags,
        buf_alloc: GUEST_BUF,
        fwd_cnt: 0,
    }
}

fn open_stream(guest: &mut GuestVsock, guest_port: u32) -> u32 {
    let port = guest.device().arm_stream().expect("a free stream port");
    guest.post_rx(4096);
    guest.send(to_stream(port, guest_port, VSOCK_OP_REQUEST, 0, 0), &[]);
    let packets = guest.recv();
    assert_eq!(packets.len(), 1, "{packets:?}");
    let (response, _) = &packets[0];
    assert_eq!(response.op, VSOCK_OP_RESPONSE);
    assert_eq!((response.src_port, response.dst_port), (port, guest_port));
    port
}

#[test]
fn an_unarmed_stream_port_refuses_a_connection_like_any_unknown_port() {
    let mut guest = GuestVsock::boot();
    guest.post_rx(4096);
    guest.send(
        to_stream(
            SOMA_STREAM_PORT_BASE,
            STREAM_GUEST_PORT,
            VSOCK_OP_REQUEST,
            0,
            0,
        ),
        &[],
    );
    let packets = guest.recv();
    assert_eq!(packets.len(), 1, "{packets:?}");
    assert_eq!(packets[0].0.op, VSOCK_OP_RST);
    assert_eq!(packets[0].0.src_port, SOMA_CONTROL_PORT);
    assert!(guest.device().stream(SOMA_STREAM_PORT_BASE).is_none());
    assert_eq!(guest.device().counters().accepted, 0);
}

#[test]
fn a_stream_and_the_control_connection_carry_bytes_without_meeting() {
    let mut guest = GuestVsock::boot();
    connect(&mut guest);
    let port = open_stream(&mut guest, STREAM_GUEST_PORT);
    assert_eq!(guest.device().counters().accepted, 2);

    guest.send(hdr(VSOCK_OP_RW, 3, 0, 0), b"cmd");
    guest.send(
        to_stream(port, STREAM_GUEST_PORT, VSOCK_OP_RW, 5, 0),
        b"frame",
    );
    let mut buf = [0u8; 16];
    let control = guest.device().endpoint().expect("control");
    assert_eq!(control.read(&mut buf), 3);
    assert_eq!(&buf[..3], b"cmd");
    let stream = guest.device().stream(port).expect("stream");
    assert_eq!(stream.read(&mut buf), 5);
    assert_eq!(&buf[..5], b"frame");

    assert_eq!(
        guest.device().stream(port).expect("stream").write(b"click"),
        5
    );
    guest.post_rx(4096);
    guest.post_rx(4096);
    let packets = guest.recv();
    // The control connection read bytes too, so its credit update goes out ahead of the stream.
    assert_eq!(packets.len(), 2, "{packets:?}");
    assert_eq!(packets[0].0.src_port, SOMA_CONTROL_PORT);
    assert_eq!(packets[0].0.op, VSOCK_OP_CREDIT_UPDATE);
    let (header, payload) = &packets[1];
    assert_eq!(header.op, VSOCK_OP_RW);
    assert_eq!(
        (header.src_port, header.dst_port),
        (port, STREAM_GUEST_PORT)
    );
    assert_eq!(payload, b"click");
    assert_eq!(
        header.fwd_cnt, 5,
        "a stream acknowledges its own bytes, not the control's"
    );
    assert_eq!(
        GUEST_PORT,
        guest.device().endpoint().expect("control").peer_port()
    );
}

#[test]
fn only_a_fixed_number_of_ports_can_be_armed_and_closing_frees_one() {
    let mut guest = GuestVsock::boot();
    let ports: Vec<u32> = (0..MAX_STREAMS)
        .map(|_| guest.device().arm_stream().expect("armed"))
        .collect();
    assert_eq!(ports[0], SOMA_STREAM_PORT_BASE);
    assert_eq!(ports.len(), MAX_STREAMS);
    assert!(guest.device().arm_stream().is_none());
    guest.device().close_stream(ports[3]);
    assert_eq!(guest.device().arm_stream(), Some(ports[3]));
}

#[test]
fn a_second_connection_to_an_open_stream_is_reset_and_the_first_survives() {
    let mut guest = GuestVsock::boot();
    let port = open_stream(&mut guest, STREAM_GUEST_PORT);
    guest.post_rx(4096);
    guest.send(
        to_stream(port, STREAM_GUEST_PORT + 1, VSOCK_OP_REQUEST, 0, 0),
        &[],
    );
    let packets = guest.recv();
    assert_eq!(packets.len(), 1, "{packets:?}");
    assert_eq!(packets[0].0.op, VSOCK_OP_RST);
    assert_eq!(packets[0].0.src_port, port);
    let stream = guest.device().stream(port).expect("stream");
    assert!(stream.is_open());
    assert_eq!(stream.peer_port(), STREAM_GUEST_PORT);
}

#[test]
fn closing_a_stream_resets_it_from_its_own_port_and_frees_the_slot() {
    let mut guest = GuestVsock::boot();
    let port = open_stream(&mut guest, STREAM_GUEST_PORT);
    guest.device().close_stream(port);
    assert!(!guest.device().is_stream_armed(port));
    guest.post_rx(4096);
    let packets = guest.recv();
    assert_eq!(packets.len(), 1, "{packets:?}");
    assert_eq!(packets[0].0.op, VSOCK_OP_RST);
    assert_eq!(
        (packets[0].0.src_port, packets[0].0.dst_port),
        (port, STREAM_GUEST_PORT)
    );
    // A late packet from the guest to the disarmed port is answered like any unknown port.
    guest.post_rx(4096);
    guest.send(
        to_stream(port, STREAM_GUEST_PORT, VSOCK_OP_REQUEST, 0, 0),
        &[],
    );
    let packets = guest.recv();
    assert_eq!(packets[0].0.op, VSOCK_OP_RST);
}

#[test]
fn a_stream_that_breaks_the_credit_contract_dies_alone() {
    let mut guest = GuestVsock::boot();
    connect(&mut guest);
    let port = open_stream(&mut guest, STREAM_GUEST_PORT);
    // Two packets that together overrun the host's 64 KiB window on the stream.
    let chunk = vec![7u8; 40_000];
    guest.post_rx(4096);
    for _ in 0..2 {
        let header = to_stream(port, STREAM_GUEST_PORT, VSOCK_OP_RW, 40_000, 0);
        guest.send(header, &chunk);
    }
    let packets = guest.recv();
    assert!(
        packets
            .iter()
            .any(|(h, _)| h.op == VSOCK_OP_RST && h.src_port == port)
    );
    assert!(!guest.device().stream(port).expect("stream").is_open());
    assert!(guest.device().endpoint().expect("control").is_open());
}

#[test]
fn streams_take_turns_and_the_control_connection_goes_first() {
    let mut guest = GuestVsock::boot();
    connect(&mut guest);
    let first = open_stream(&mut guest, STREAM_GUEST_PORT);
    let second = open_stream(&mut guest, STREAM_GUEST_PORT + 1);
    for port in [first, second] {
        assert_eq!(guest.device().stream(port).expect("stream").write(b"ab"), 2);
    }
    assert_eq!(guest.device().endpoint().expect("control").write(b"c"), 1);
    for _ in 0..3 {
        guest.post_rx(4096);
    }
    let sources: Vec<u32> = guest.recv().iter().map(|(h, _)| h.src_port).collect();
    assert_eq!(sources, [SOMA_CONTROL_PORT, first, second]);
}

#[test]
fn a_reset_or_restore_drops_every_stream_and_a_stream_blocks_capture() {
    let mut guest = GuestVsock::boot();
    let port = open_stream(&mut guest, STREAM_GUEST_PORT);
    assert!(!guest.device().is_quiescent());
    let state = guest.device().snapshot_state();
    guest.device().restore_state(&state).expect("restore");
    assert!(!guest.device().is_stream_armed(port));
    assert!(guest.device().stream(port).is_none());
    let _ = guest.device().arm_stream().expect("slots are free again");
}

#[test]
fn random_packets_across_the_control_port_and_streams_never_fault_the_device() {
    let mut guest = GuestVsock::boot();
    let ports: Vec<u32> = (0..3)
        .map(|_| guest.device().arm_stream().expect("armed"))
        .collect();
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for round in 0..600_u32 {
        let pick = next() % 6;
        let dst = match pick {
            0 => SOMA_CONTROL_PORT,
            1..=3 => ports[usize::try_from(pick - 1).expect("small")],
            4 => SOMA_STREAM_PORT_BASE + u32::try_from(next() % 12).expect("small"),
            _ => u32::try_from(next() % 5000).expect("small"),
        };
        let mut header = to_stream(dst, STREAM_GUEST_PORT + round % 3, 0, 0, 0);
        header.op = u16::try_from(next() % 9).expect("small");
        header.buf_alloc = u32::try_from(next() & 0xffff).expect("small");
        header.fwd_cnt = u32::try_from(next() & 0xff).expect("small");
        let payload_len = usize::try_from(next() % 200).expect("small");
        header.len = u32::try_from(payload_len).expect("small");
        guest.send(header, &vec![7_u8; payload_len]);
        // Post a buffer only while the device has something to send, so the queue never fills.
        while guest.device().has_outbound() && guest.device().pending_events() == 0 {
            guest.post_rx(4096);
            if guest.recv().is_empty() {
                break;
            }
        }
        if round % 40 == 0 {
            for port in &ports {
                if let Some(endpoint) = guest.device().stream(*port) {
                    let mut buf = [0u8; 128];
                    let _ = endpoint.read(&mut buf);
                    let _ = endpoint.write(&buf);
                }
            }
        }
        assert!(guest.t.is_active());
    }
}
