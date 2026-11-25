//! A call to a Host that restarted is made once more on a fresh connection (SOMA#241).
//!
//! A restarted Host closes every connection opened before it, and a machine host keeps the one
//! it opened at start. The first attempt below fails the way a call on that closed connection
//! does; the second is made on a fresh connection to a real daemon.

use std::cell::Cell;

use soma_hostd::{FailureCode, client::HostClient};

use super::*;
use crate::backend::kvm::runtime::again_if_restarted;

#[test]
fn a_call_that_failed_in_transport_is_made_once_more_on_a_fresh_connection() {
    let host = Host::serve();
    let client = HostClient::connect(host.socket()).expect("connect");
    let attempts = Cell::new(0);
    let answered = again_if_restarted(&client, host.socket(), |fresh| {
        attempts.set(attempts.get() + 1);
        if attempts.get() == 1 {
            Err(ClientError::Receive(0))
        } else {
            fresh.list(None).map(|page| page.instances.len())
        }
    });
    assert_eq!(answered, Ok(0), "the second attempt reached the Host");
    assert_eq!(attempts.get(), 2);
}

#[test]
fn a_refusal_by_the_host_is_not_made_again() {
    let host = Host::serve();
    let client = HostClient::connect(host.socket()).expect("connect");
    let attempts = Cell::new(0);
    let answered: Result<(), ClientError> = again_if_restarted(&client, host.socket(), |_| {
        attempts.set(attempts.get() + 1);
        Err(ClientError::Refused(FailureCode::Unknown))
    });
    assert_eq!(answered, Err(ClientError::Refused(FailureCode::Unknown)));
    assert_eq!(attempts.get(), 1, "an answer is not a transport failure");
}
