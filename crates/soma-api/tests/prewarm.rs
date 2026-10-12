//! The prewarm routes: what a caller can read, and what a bad set is told.
//!
//! The pool itself needs a prepared Generation and KVM; these prove the surface and its
//! refusals, which hold on any host. What a host with a store does is proved live.

#[allow(
    dead_code,
    reason = "each integration test uses a subset of the shared fixtures"
)]
mod support;

use support::{FakeFacade, Mode, call, identified};

#[test]
fn the_desired_set_and_what_exists_can_be_read() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let (status, body) = call(&mut facade, &identified("GET", "/v1/prewarm", ""));

    assert_eq!(status, 200);
    assert_eq!(body["operation"], "prewarm.status");
    let result = &body["result"];
    assert!(result["targets"].is_array());
    assert!(result["uncovered_demand"].is_array());
    for field in [
        "source",
        "budget_mib",
        "committed_mib",
        "unprimed_target",
        "unprimed_prepared",
    ] {
        assert!(!result[field].is_null(), "{field} is part of the readback");
    }
    assert!(facade.calls.is_empty(), "prewarm never touches a sandbox");
}

#[test]
fn a_set_must_be_a_document_of_targets_with_known_fields() {
    for body in [
        "",
        "not json",
        "{}",
        r#"{"targets":"node:22"}"#,
        r#"{"targets":[{"reference":"node:22"}]}"#,
        r#"{"targets":[{"reference":"node:22","count":-1}]}"#,
        r#"{"targets":[{"reference":"node:22","count":1,"surprise":true}]}"#,
        r#"{"targets":[],"extra":1}"#,
    ] {
        let mut facade = FakeFacade::new(Mode::Succeed);
        let (status, answer) = call(&mut facade, &identified("PUT", "/v1/prewarm", body));
        assert_eq!(status, 400, "{body}");
        assert_eq!(answer["error"]["code"], "invalid_input", "{body}");
    }
}

#[test]
fn a_host_that_prepares_nothing_refuses_a_set_by_name() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let (status, answer) = call(
        &mut facade,
        &identified(
            "PUT",
            "/v1/prewarm",
            r#"{"targets":[{"reference":"node:22","count":2}]}"#,
        ),
    );

    assert_eq!(status, 501);
    assert_eq!(answer["error"]["code"], "prewarm_unsupported");
    assert_eq!(answer["error"]["retryable"], false);
}

#[test]
fn only_get_and_put_are_accepted_and_the_caller_must_be_identified() {
    for method in ["POST", "DELETE", "PATCH"] {
        let mut facade = FakeFacade::new(Mode::Succeed);
        let (status, answer) = call(&mut facade, &identified(method, "/v1/prewarm", "{}"));
        assert_eq!(status, 405, "{method}");
        assert_eq!(answer["error"]["code"], "method_not_allowed");
    }
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = soma_api::http::request::Request::read_from(&mut std::io::BufReader::new(
        &b"GET /v1/prewarm HTTP/1.1\r\nhost: x\r\n\r\n"[..],
    ))
    .expect("a request without a tenant header");
    let response = soma_api::handle(&mut facade, &request);
    assert_ne!(response.status, 200, "an unidentified caller is refused");
}
