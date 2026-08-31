//! What the transport does when there is no machine behind it.
//!
//! Every case here is about the seam rather than the sandbox: a name with nothing serving it, a
//! socket something already answers on, a line that will not fit, and a refusal that has to mean
//! the same thing on both sides of a process boundary. None of it needs KVM, which is the point:
//! these are the paths a caller reaches when the machine is exactly what is missing.

use std::{io::BufReader, os::unix::net::UnixListener};

use soma::{BackendFailureKind, InstanceId};

use super::{
    channel,
    wire::{Call, Refusal},
};

fn instance(value: &str) -> InstanceId {
    InstanceId::new(value).expect("a 32 character lowercase hexadecimal identity")
}

const ONE: &str = "0123456789abcdef0123456789abcdef";
const TWO: &str = "fedcba9876543210fedcba9876543210";

#[test]
fn every_refusal_means_the_same_kind_on_both_sides() {
    let kinds = [
        BackendFailureKind::Unsupported,
        BackendFailureKind::Unavailable,
        BackendFailureKind::ResourceConflict,
        BackendFailureKind::WorkloadRejected,
        BackendFailureKind::IsolationFailure,
        BackendFailureKind::GuestFailure,
        BackendFailureKind::Timeout,
        BackendFailureKind::OutputLimit,
        BackendFailureKind::CleanupFailure,
    ];
    for kind in kinds {
        let encoded = serde_json::to_vec(&Refusal::from(kind)).expect("a refusal encodes");
        let decoded: Refusal = serde_json::from_slice(&encoded).expect("a refusal decodes");
        assert_eq!(
            BackendFailureKind::from(decoded),
            kind,
            "a refusal must not change kind on the way across"
        );
    }
}

#[test]
fn an_instance_with_no_host_is_absent_rather_than_broken() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    assert!(
        channel::connect(directory.path(), &instance(ONE)).is_err(),
        "nothing serves this Instance, so no connection may be reported"
    );
}

#[test]
fn a_socket_nothing_answers_on_is_removed_by_the_lookup_that_finds_it() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let socket = channel::socket_path(directory.path(), &instance(ONE));
    // A host that was killed leaves its socket behind; the next lookup must not report it as one.
    drop(UnixListener::bind(&socket).expect("a socket to abandon"));
    assert!(socket.exists(), "the abandoned socket exists to begin with");

    assert!(channel::connect(directory.path(), &instance(ONE)).is_err());
    assert!(
        !socket.exists(),
        "a socket no process answers on must not survive the lookup that found it"
    );
}

#[test]
fn binding_refuses_a_socket_something_already_answers_on() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let socket = channel::socket_path(directory.path(), &instance(TWO));
    let held = channel::bind(&socket).expect("the first host binds");

    assert!(
        channel::bind(&socket).is_err(),
        "a second host must not take over an Instance another is serving"
    );
    drop(held);

    // Once the first host is gone the name is free again, which is what makes a killed host
    // recoverable rather than a permanently poisoned identity.
    assert!(channel::bind(&socket).is_ok());
}

#[test]
fn one_line_carries_one_call() {
    let mut written = Vec::new();
    let call = Call::Cleanup {
        instance_id: instance(ONE),
        forced: true,
    };
    channel::write_line(&mut written, &call).expect("a call encodes");
    let (terminator, body) = written.split_last().expect("an encoded call is not empty");
    assert_eq!(*terminator, b'\n', "a call ends at a newline");
    assert!(
        !body.contains(&b'\n'),
        "a call is exactly one line, because the reader stops at the first newline"
    );

    let mut reader = BufReader::new(written.as_slice());
    let decoded = channel::read_line::<Call>(&mut reader).expect("a call decodes");
    assert!(matches!(
        decoded,
        Call::Cleanup { forced: true, .. } // the release method has to survive the crossing
    ));
}

#[test]
fn a_line_that_is_not_a_call_is_refused_rather_than_guessed_at() {
    let mut reader = BufReader::new(&b"{\"execute\":{}}\n"[..]);
    assert!(
        channel::read_line::<Call>(&mut reader).is_none(),
        "a malformed line must not become some other call"
    );

    let mut empty = BufReader::new(&b""[..]);
    assert!(
        channel::read_line::<Call>(&mut empty).is_none(),
        "a peer that is gone says nothing rather than something"
    );
}
