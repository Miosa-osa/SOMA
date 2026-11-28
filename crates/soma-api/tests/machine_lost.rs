//! A sandbox whose machine is gone reads as stopped, with the reason (SOMA#220).
//!
//! The platform restarts an always-on sandbox it reads as stopped, which is how a crashed
//! desktop gets back up.

#[allow(
    dead_code,
    reason = "each integration test uses a subset of the shared fixtures"
)]
mod support;

use support::{FIXTURE_INSTANCE_ID, FakeFacade, Mode, call, identified};

#[test]
fn a_sandbox_whose_machine_is_gone_reads_as_stopped_with_its_reason() {
    let mut facade = FakeFacade::new(Mode::Lost);
    let request = identified("GET", &format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}"), "");

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 200);
    assert_eq!(body["result"]["state"], "stopped");
    assert_eq!(body["result"]["reason"], "machine_lost");
}

#[test]
fn a_running_sandbox_reads_with_no_reason() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified("GET", &format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}"), "");

    let (_, body) = call(&mut facade, &request);

    assert_eq!(body["result"]["state"], "ready");
    assert!(body["result"].get("reason").is_none());
}

/// SOMA#258: a guest shut down from inside reads as stopped, and says the guest did it, so the
/// platform tells a user's Shut down from a crash.
#[test]
fn a_sandbox_its_guest_shut_down_reads_as_stopped_by_the_guest() {
    let mut facade = FakeFacade::new(Mode::ShutDownByGuest);
    let request = identified("GET", &format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}"), "");

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 200);
    assert_eq!(body["result"]["state"], "stopped");
    assert_eq!(body["result"]["reason"], "guest_shutdown");
}
