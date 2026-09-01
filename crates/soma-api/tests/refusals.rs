#[allow(
    dead_code,
    reason = "each integration test uses a subset of the shared fixtures"
)]
mod support;

use soma::{BackendKind, InstanceId, SandboxEntry, SandboxLiveness, SandboxPhase};
use support::{Call, FIXTURE_INSTANCE_ID, FakeFacade, Mode, call, identified};

#[test]
fn listing_sandboxes_reports_durable_state_and_host_liveness_separately() {
    let entry = SandboxEntry::new(
        InstanceId::new(FIXTURE_INSTANCE_ID).expect("fixture identity is canonical"),
        SandboxPhase::Active,
        BackendKind::LinuxKvm,
        None,
        SandboxLiveness::Absent,
    );
    let mut facade = FakeFacade::new(Mode::Succeed).holding(vec![entry]);
    let request = identified("GET", "/v1/sandboxes", "");

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 200);
    assert_eq!(facade.calls, [Call::List]);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["operation"], "sandbox.list");
    assert_eq!(body["result"]["count"], 1);
    assert_eq!(body["result"]["sandboxes"][0]["state"], "active");
    assert_eq!(body["result"]["sandboxes"][0]["host"], "absent");
}

#[test]
fn an_empty_listing_is_an_explicit_successful_collection() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified("GET", "/v1/sandboxes", "");

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 200);
    assert_eq!(body["result"]["count"], 0);
    assert_eq!(body["result"]["sandboxes"], serde_json::json!([]));
    assert!(body["receipt"].is_null());
}

/// A backend that keeps no machine past its launching process still refuses, and names why.
///
/// The refusal is what a macOS host answers: the route and the engine call both exist now, so the
/// only thing missing is a sandbox for the call to reach, and that is what the message must say.
#[test]
fn a_backend_holding_no_durable_machine_names_the_missing_capability() {
    for operation in ["read", "write", "list", "exists", "remove", "mkdir"] {
        let mut facade = FakeFacade::new(Mode::Unsupported);
        let request = identified(
            "POST",
            &format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}/filesystem/{operation}"),
            &body_for(operation),
        );

        let (status, body) = call(&mut facade, &request);

        assert_eq!(status, 501, "{operation} must refuse");
        assert_eq!(body["operation"], "sandbox.filesystem");
        assert_eq!(body["error"]["code"], "capability_unavailable");
        assert!(
            body["error"]["message"]
                .as_str()
                .expect("the failure carries a message")
                .contains("no sandbox left to address")
        );
    }
}

/// The message must no longer describe the facade as having no path to the guest at all.
#[test]
fn the_capability_message_no_longer_names_a_gap_that_is_closed() {
    let message = soma_api::MissingCapability::GuestFilesystemTransfer.message();

    assert!(!message.contains("exposes no guest filesystem transfer"));
    assert!(!message.contains("no backend or engine method reaches it"));
}

/// The body a given operation needs, so every route is reached rather than refused for shape.
fn body_for(operation: &str) -> String {
    if operation == "write" {
        r#"{"path":"/workspace/main.js","content":"aGk="}"#.to_owned()
    } else {
        r#"{"path":"/workspace/main.js"}"#.to_owned()
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
