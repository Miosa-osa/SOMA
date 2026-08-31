#[allow(
    dead_code,
    reason = "each integration test uses a subset of the shared fixtures"
)]
mod support;

use support::{FIXTURE_INSTANCE_ID, FakeFacade, Mode, call, identified};

#[test]
fn listing_sandboxes_refuses_and_names_the_missing_store_capability() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified("GET", "/v1/sandboxes", "");

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 501);
    assert!(facade.calls.is_empty());
    assert_eq!(body["status"], "error");
    assert_eq!(body["operation"], "sandbox.list");
    assert_eq!(body["error"]["code"], "capability_unavailable");
    assert_eq!(body["error"]["retryable"], false);
    assert!(
        body["error"]["message"]
            .as_str()
            .expect("the failure carries a message")
            .contains("cannot enumerate sandboxes")
    );
}

#[test]
fn listing_sandboxes_never_answers_with_an_empty_collection() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified("GET", "/v1/sandboxes", "");

    let (_, body) = call(&mut facade, &request);

    assert!(body["result"].is_null());
    assert!(body["receipt"].is_null());
}

#[test]
fn every_filesystem_operation_refuses_and_names_the_missing_guest_capability() {
    for operation in ["read", "write", "list", "remove", "mkdir"] {
        let mut facade = FakeFacade::new(Mode::Succeed);
        let request = identified(
            "POST",
            &format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}/filesystem/{operation}"),
            r#"{"path":"/workspace/main.js"}"#,
        );

        let (status, body) = call(&mut facade, &request);

        assert_eq!(status, 501, "{operation} must refuse");
        assert!(
            facade.calls.is_empty(),
            "{operation} must not reach the engine"
        );
        assert_eq!(body["operation"], "sandbox.filesystem");
        assert_eq!(body["error"]["code"], "capability_unavailable");
        assert!(
            body["error"]["message"]
                .as_str()
                .expect("the failure carries a message")
                .contains("exposes no guest filesystem transfer")
        );
    }
}

#[test]
fn an_unknown_filesystem_operation_is_not_found_rather_than_unimplemented() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified(
        "POST",
        &format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}/filesystem/chown"),
        "{}",
    );

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 404);
    assert_eq!(body["error"]["code"], "route_not_found");
}

#[test]
fn an_unknown_route_is_refused_without_touching_the_engine() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified("GET", "/v1/templates", "");

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 404);
    assert!(facade.calls.is_empty());
    assert_eq!(body["error"]["code"], "route_not_found");
}

#[test]
fn a_known_path_under_the_wrong_method_reports_the_method_rather_than_absence() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified("DELETE", "/v1/sandboxes", "");

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 405);
    assert!(facade.calls.is_empty());
    assert_eq!(body["error"]["code"], "method_not_allowed");
}

#[test]
fn creating_a_sandbox_no_later_request_could_address_is_refused_before_it_is_built() {
    let mut facade = FakeFacade::new(Mode::Succeed).without_addressable_sandboxes();
    let request = identified("POST", "/v1/sandboxes", r#"{"image":"node:22"}"#);

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 501);
    assert!(
        facade.calls.is_empty(),
        "nothing is launched for an identity the caller could never use"
    );
    assert_eq!(body["status"], "error");
    assert_eq!(body["operation"], "sandbox.create");
    assert_eq!(body["error"]["code"], "capability_unavailable");
    assert_eq!(body["error"]["retryable"], false);
    assert!(
        body["error"]["message"]
            .as_str()
            .expect("the failure carries a message")
            .contains("hosts a machine inside the process that launched it")
    );
}
