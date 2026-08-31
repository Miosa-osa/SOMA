//! What the client must accept, and every shape of answer it must refuse.

use std::os::fd::AsFd as _;

use super::*;
use crate::{
    BundleId, CleanupGeneration, EgressClass, InstanceId, IntentDigest, NetworkIntent, OperationId,
    ProfileDigest, error_code, send_tap, seqpacket_pair,
};
use soma_guest::ActivationChallenge;

fn intent() -> NetworkIntent {
    NetworkIntent::new(
        EgressClass::PublicInternet,
        Vec::new(),
        Vec::new(),
        ProfileDigest([1; 32]),
    )
    .expect("intent")
}

fn claim() -> Request {
    Request::Claim {
        instance: InstanceId::new([1; 16]).expect("instance"),
        operation: OperationId::new([2; 16]).expect("operation"),
        vsock_cid: 7,
        intent: intent(),
    }
}

fn claimed(bundle: BundleId, generation: CleanupGeneration) -> Reply {
    Reply::Claimed {
        bundle,
        generation,
        launch: [9; 35],
        activation: ActivationChallenge::from_bytes([4; 32]).expect("challenge"),
    }
}

fn header(bundle: BundleId, generation: CleanupGeneration) -> TransferHeader {
    TransferHeader {
        bundle,
        generation,
        intent: IntentDigest([9; 32]),
    }
}

/// Answers the client's request from a connected peer, then returns what the client made of it.
fn exchange(answer: impl FnOnce(&OwnedFd)) -> Result<(Reply, Option<OwnedFd>), ClientError> {
    let (client, broker) = seqpacket_pair().expect("pair");
    answer(&broker);
    // The peer stops writing once it has answered, so a client waiting for a packet that is
    // never coming observes the end of the stream instead of blocking this test forever. The
    // socket stays open, so the client's own request is still accepted, and packets already
    // queued stay readable.
    // SAFETY: `shutdown` takes no pointer argument.
    unsafe { libc::shutdown(broker.as_raw_fd(), libc::SHUT_WR) };
    BrokerClient { socket: client }.call(&claim())
}

fn reply_only(broker: &OwnedFd, reply: &Reply) {
    let bytes = reply.encode();
    // SAFETY: `bytes` is a valid buffer for its full length.
    let sent = unsafe { libc::send(broker.as_raw_fd(), bytes.as_ptr().cast(), bytes.len(), 0) };
    assert_eq!(usize::try_from(sent), Ok(bytes.len()));
}

#[test]
fn a_path_with_no_broker_listening_is_unreachable() {
    let directory = tempfile::tempdir().expect("directory");
    assert_eq!(
        BrokerClient::connect(&directory.path().join("absent.sock")).expect_err("no broker"),
        ClientError::Unreachable
    );
}

#[test]
fn an_accepted_claim_yields_the_reply_and_a_nonblocking_descriptor() {
    let bundle = BundleId::new([3; 16]).expect("bundle");
    let generation = CleanupGeneration::new(2).expect("generation");
    let (reply, descriptor) = exchange(|broker| {
        let file = tempfile::tempfile().expect("file");
        send_tap(broker.as_fd(), &header(bundle, generation), file.as_fd()).expect("sent");
        reply_only(broker, &claimed(bundle, generation));
    })
    .expect("accepted");
    assert_eq!(reply, claimed(bundle, generation));
    let descriptor = descriptor.expect("descriptor");
    // SAFETY: `F_GETFL` takes no pointer argument.
    let flags = unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_GETFL) };
    assert_ne!(flags & libc::O_NONBLOCK, 0);
}

#[test]
fn a_refusal_carries_the_brokers_own_failure_code() {
    let code = error_code(&crate::Error::PoolExhausted);
    assert_eq!(
        exchange(|broker| reply_only(broker, &Reply::Failed(code))).expect_err("refused"),
        ClientError::Refused(code)
    );
}

/// A descriptor and a reply that disagree cannot both describe this Instance's lease.
#[test]
fn a_descriptor_from_one_assignment_with_a_reply_from_another_is_refused() {
    let bundle = BundleId::new([3; 16]).expect("bundle");
    let other = BundleId::new([4; 16]).expect("other");
    let generation = CleanupGeneration::new(2).expect("generation");
    let error = exchange(|broker| {
        let file = tempfile::tempfile().expect("file");
        send_tap(broker.as_fd(), &header(bundle, generation), file.as_fd()).expect("sent");
        reply_only(broker, &claimed(other, generation));
    })
    .expect_err("mismatched");
    assert_eq!(error, ClientError::Protocol);
}

/// A broker that closes without answering leaves no reply to misread as an assignment.
#[test]
fn a_broker_that_answers_nothing_is_a_protocol_failure() {
    let error = exchange(|_| ()).expect_err("no answer");
    assert_eq!(error, ClientError::Protocol);
}
