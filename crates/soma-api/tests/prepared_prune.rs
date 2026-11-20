//! The prune route: what a caller must send, and what a host with no usable store answers.
//!
//! What a prune does to a prepared root is proved in `soma-local` against real directories; these
//! prove the surface and its refusals, which hold on any host.

#[allow(
    dead_code,
    reason = "each integration test uses a subset of the shared fixtures"
)]
mod support;

use support::{Call, FakeFacade, Mode, anonymous, call, identified};

const PRUNE: &str = "/v1/prepared/prune";

#[test]
fn an_applied_prune_must_name_what_it_keeps() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let (status, answer) = call(&mut facade, &identified("POST", PRUNE, r#"{"apply":true}"#));

    assert_eq!(status, 422);
    assert_eq!(answer["operation"], "prepared.prune");
    assert_eq!(answer["error"]["code"], "keep_required");
    assert!(facade.calls.is_empty(), "a refused request reads nothing");
}

#[test]
fn a_kept_value_that_is_not_a_reference_is_named() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let (status, answer) = call(
        &mut facade,
        &identified("POST", PRUNE, r#"{"keep":["has space:1"],"apply":true}"#),
    );

    assert_eq!(status, 422);
    assert_eq!(answer["error"]["code"], "invalid_keep");
    assert!(answer["result"]["reason"].is_string());
}

#[test]
fn a_body_must_be_a_prune_document_with_known_fields() {
    for body in [
        "not json",
        r#"{"keep":"node:22"}"#,
        r#"{"apply":"yes"}"#,
        r#"{"keep":[],"surprise":true}"#,
    ] {
        let mut facade = FakeFacade::new(Mode::Succeed);
        let (status, answer) = call(&mut facade, &identified("POST", PRUNE, body));
        assert_eq!(status, 400, "{body}");
        assert_eq!(answer["error"]["code"], "invalid_input", "{body}");
    }
}

/// A test process names no prepared store, so a dry run is refused by name rather than planned.
#[test]
fn a_host_with_no_prepared_store_refuses_a_dry_run_by_name() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let (status, answer) = call(&mut facade, &identified("POST", PRUNE, ""));

    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        assert_eq!(status, 503);
        assert_eq!(answer["error"]["code"], "prune_store_unavailable");
        assert!(answer["result"]["reason"].is_string());
    } else {
        assert_eq!(status, 501);
        assert_eq!(answer["error"]["code"], "prune_unsupported");
    }
    assert!(
        !facade.calls.contains(&Call::Launch),
        "a prune never launches anything"
    );
}

#[test]
fn only_post_is_accepted_and_the_caller_must_be_identified() {
    for method in ["GET", "PUT", "DELETE"] {
        let mut facade = FakeFacade::new(Mode::Succeed);
        let (status, answer) = call(&mut facade, &identified(method, PRUNE, ""));
        assert_eq!(status, 405, "{method}");
        assert_eq!(answer["error"]["code"], "method_not_allowed");
    }
    let mut facade = FakeFacade::new(Mode::Succeed);
    let (status, _) = call(&mut facade, &anonymous("POST", PRUNE, ""));
    assert_eq!(status, 401, "an unidentified caller is refused");
}
