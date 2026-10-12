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

/// The prewarm routes are served by the connection, without a facade.
fn prewarm_call(raw: &str) -> (u16, serde_json::Value) {
    let mut reader = std::io::Cursor::new(raw.as_bytes().to_vec());
    let request =
        soma_api::http::request::Request::read_from(&mut reader).expect("the fixture parses");
    let response = soma_api::handle_prewarm(&request);
    (
        response.status,
        serde_json::from_slice(&response.body).expect("a JSON envelope"),
    )
}

#[test]
fn the_desired_set_and_what_exists_can_be_read() {
    let (status, body) = prewarm_call(&identified("GET", "/v1/prewarm", ""));

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
        let (status, answer) = prewarm_call(&identified("PUT", "/v1/prewarm", body));
        assert_eq!(status, 400, "{body}");
        assert_eq!(answer["error"]["code"], "invalid_input", "{body}");
    }
}

#[test]
fn a_host_that_prepares_nothing_refuses_a_set_by_name() {
    let (status, answer) = prewarm_call(&identified(
        "PUT",
        "/v1/prewarm",
        r#"{"targets":[{"reference":"node:22","count":2}]}"#,
    ));

    assert_eq!(status, 501);
    assert_eq!(answer["error"]["code"], "prewarm_unsupported");
    assert_eq!(answer["error"]["retryable"], false);
}

#[test]
fn only_get_and_put_are_accepted_and_the_caller_must_be_identified() {
    for method in ["POST", "DELETE", "PATCH"] {
        let (status, answer) = prewarm_call(&identified(method, "/v1/prewarm", "{}"));
        assert_eq!(status, 405, "{method}");
        assert_eq!(answer["error"]["code"], "method_not_allowed");
    }
    let (status, _) = prewarm_call("GET /v1/prewarm HTTP/1.1\r\nhost: x\r\n\r\n");
    assert_ne!(status, 200, "an unidentified caller is refused");
}

#[test]
fn the_sandbox_handler_never_serves_the_prewarm_routes() {
    // Only the connection path may change what is prepared; `handle` has no pool to speak for.
    for (method, body) in [("GET", ""), ("PUT", r#"{"targets":[]}"#)] {
        let mut facade = FakeFacade::new(Mode::Succeed);
        let (status, answer) = call(&mut facade, &identified(method, "/v1/prewarm", body));
        assert_eq!(status, 404, "{method}");
        assert_eq!(answer["error"]["code"], "route_not_found");
        assert!(facade.calls.is_empty(), "prewarm never touches a sandbox");
    }
}
