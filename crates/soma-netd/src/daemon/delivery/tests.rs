//! Delivery gates: a reply is complete or it is a terminal protocol failure.
//!
//! The fixtures use real connected `SOCK_SEQPACKET` sockets, so they exercise the exact send
//! the daemon performs rather than a stub.

use std::os::fd::{AsFd, AsRawFd};

use super::deliver;
use crate::{BundleId, CleanupGeneration, Error, Reply, seqpacket_pair};

/// The largest number of unread replies the test will queue before it declares that the send
/// blocked instead of refusing.
const PRESSURE: usize = 4096;

fn reply() -> Reply {
    Reply::Claimed {
        bundle: BundleId::new([3; 16]).expect("bundle"),
        generation: CleanupGeneration::new(2).expect("generation"),
        launch: [9; 35],
        activation: soma_guest::ActivationChallenge::from_bytes([4; 32]).expect("challenge"),
    }
}

fn receive(socket: &std::os::fd::OwnedFd) -> Option<Reply> {
    let mut frame = [0_u8; crate::MAX_FRAME + 1];
    // SAFETY: `frame` is a valid writable buffer of exactly the passed length.
    let received = unsafe {
        libc::recv(
            socket.as_raw_fd(),
            frame.as_mut_ptr().cast(),
            frame.len(),
            0,
        )
    };
    if received <= 0 {
        return None;
    }
    let length = usize::try_from(received).expect("a nonnegative length");
    Some(Reply::decode(&frame[..length]).expect("a complete reply"))
}

#[test]
fn a_complete_reply_reaches_the_peer_exactly_once() {
    let (broker, peer) = seqpacket_pair().expect("pair");

    deliver(broker.as_fd(), &reply().encode()).expect("delivered");

    assert_eq!(receive(&peer), Some(reply()));
}

#[test]
fn a_reply_to_a_departed_peer_is_a_terminal_protocol_failure() {
    let (broker, peer) = seqpacket_pair().expect("pair");
    drop(peer);

    assert_eq!(
        deliver(broker.as_fd(), &reply().encode()),
        Err(Error::Protocol("reply delivery"))
    );
}

#[test]
fn a_peer_that_stops_reading_refuses_delivery_rather_than_blocking_the_broker() {
    let (broker, peer) = seqpacket_pair().expect("pair");
    let bytes = reply().encode();

    let mut delivered = 0;
    while delivered < PRESSURE && deliver(broker.as_fd(), &bytes).is_ok() {
        delivered += 1;
    }

    assert!(
        delivered < PRESSURE,
        "the send never refused; a peer that stops reading can wedge the broker"
    );
    assert_eq!(
        deliver(broker.as_fd(), &bytes),
        Err(Error::Protocol("reply delivery"))
    );
    // The replies the peer did receive are complete frames, so the refusal is a boundary
    // between delivered and undelivered rather than a partial write.
    assert_eq!(receive(&peer), Some(reply()));
}
