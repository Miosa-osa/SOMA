#[allow(
    dead_code,
    reason = "each integration test uses a subset of the shared fixtures"
)]
mod support;

use support::{FIXTURE_INSTANCE_ID, FakeFacade, Mode, anonymous, call, identified};

/// Every route the service publishes, so a new route cannot be added without also being proved
/// to fail closed.
fn every_route() -> Vec<(&'static str, String)> {
    vec![
        ("POST", "/v1/sandboxes".to_owned()),
        ("GET", "/v1/sandboxes".to_owned()),
        ("GET", format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}")),
        ("DELETE", format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}")),
        ("POST", format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}/stop")),
        (
            "POST",
            format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}/commands"),
        ),
        (
            "POST",
            format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}/filesystem/read"),
        ),
    ]
}

#[test]
fn every_route_rejects_a_request_that_carries_no_identity() {
    for (method, path) in every_route() {
        let mut facade = FakeFacade::new(Mode::Succeed);
        let request = anonymous(method, &path, r#"{"image":"node:22"}"#);

        let (status, body) = call(&mut facade, &request);

        assert_eq!(status, 401, "{method} {path} must refuse");
        assert!(
            facade.calls.is_empty(),
            "{method} {path} must not reach the engine"
        );
        assert_eq!(body["status"], "error");
        assert_eq!(body["error"]["code"], "identity_required");
    }
}

#[test]
fn an_unidentified_request_to_an_unknown_route_is_still_unauthorized() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = anonymous("GET", "/v1/does-not-exist", "");

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 401);
    assert_eq!(body["error"]["code"], "identity_required");
}

#[test]
fn an_identity_outside_the_accepted_grammar_is_refused() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let raw = concat!(
        "GET /v1/sandboxes/89db112753324c3e890ef78b74381aa5 HTTP/1.1\r\n",
        "host: localhost\r\n",
        "x-soma-tenant: Acme Corp\r\n",
        "content-length: 0\r\n\r\n",
    );

    let (status, body) = call(&mut facade, raw);

    assert_eq!(status, 400);
    assert!(facade.calls.is_empty());
    assert_eq!(body["error"]["code"], "invalid_identity");
}

#[test]
fn an_empty_identity_header_is_refused_rather_than_treated_as_absent() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let raw = concat!(
        "GET /v1/sandboxes/89db112753324c3e890ef78b74381aa5 HTTP/1.1\r\n",
        "x-soma-tenant:\r\n",
        "content-length: 0\r\n\r\n",
    );

    let (status, body) = call(&mut facade, raw);

    assert_eq!(status, 400);
    assert_eq!(body["error"]["code"], "invalid_identity");
}

#[test]
fn the_identity_header_is_matched_without_regard_to_case() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let raw = concat!(
        "GET /v1/sandboxes/89db112753324c3e890ef78b74381aa5 HTTP/1.1\r\n",
        "X-Soma-Tenant: acme\r\n",
        "content-length: 0\r\n\r\n",
    );

    let (status, _) = call(&mut facade, raw);

    assert_eq!(status, 200);
}

#[test]
fn an_identified_request_reaches_the_engine() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified("GET", &format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}"), "");

    let (status, _) = call(&mut facade, &request);

    assert_eq!(status, 200);
    assert_eq!(facade.calls.len(), 1);
}
