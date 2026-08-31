#[allow(
    dead_code,
    reason = "each integration test uses a subset of the shared fixtures"
)]
mod support;

use support::{Call, FIXTURE_INSTANCE_ID, FakeFacade, Mode, call, identified};

#[test]
fn creating_a_sandbox_answers_created_with_the_launch_receipt() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified(
        "POST",
        "/v1/sandboxes",
        r#"{"image":"node:22","shape":{"vcpu_count":1,"memory_mib":1024,"storage_mib":10240,"capabilities":{"network":{"profile":{"mode":"disabled"},"guest_addresses":{"ipv4":{"mode":"disabled"},"ipv6":{"mode":"disabled"}},"proxy":{"mode":"disabled"},"egress":"denied","dns":{"mode":"denied"},"published_ports":[]}}}}"#,
    );

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 201);
    assert_eq!(facade.calls, vec![Call::Launch]);
    assert_eq!(body["schema"], "soma.api.v1");
    assert_eq!(body["operation"], "sandbox.create");
    assert_eq!(body["status"], "ok");
    assert_eq!(body["result"]["instance_id"], FIXTURE_INSTANCE_ID);
    assert_eq!(body["result"]["state"], "ready");
    assert_eq!(body["receipt"]["instance_id"], FIXTURE_INSTANCE_ID);
}

#[test]
fn creating_a_sandbox_without_a_shape_uses_the_facade_defaults() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified("POST", "/v1/sandboxes", r#"{"image":"node:22"}"#);

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 201);
    assert_eq!(body["status"], "ok");
}

#[test]
fn creating_a_sandbox_with_an_unparseable_image_never_reaches_the_engine() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified("POST", "/v1/sandboxes", r#"{"image":"https://node:22"}"#);

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 400);
    assert!(facade.calls.is_empty());
    assert_eq!(body["error"]["code"], "invalid_input");
}

#[test]
fn creating_a_sandbox_with_an_unknown_field_is_refused_rather_than_ignored() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified(
        "POST",
        "/v1/sandboxes",
        r#"{"image":"node:22","metadata":{"team":"platform"}}"#,
    );

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 400);
    assert!(facade.calls.is_empty());
    assert_eq!(body["error"]["code"], "invalid_input");
}

#[test]
fn getting_a_sandbox_reports_its_observed_state_and_backend() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified("GET", &format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}"), "");

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 200);
    assert_eq!(facade.calls, vec![Call::Inspect]);
    assert_eq!(body["operation"], "sandbox.get");
    assert_eq!(body["result"]["state"], "ready");
    assert_eq!(body["result"]["backend"], "linux_kvm");
}

#[test]
fn getting_an_absent_sandbox_reports_the_facade_not_found_code() {
    let mut facade = FakeFacade::new(Mode::NotFound);
    let request = identified("GET", &format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}"), "");

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 404);
    assert_eq!(body["status"], "error");
    assert_eq!(body["error"]["code"], "machine_not_found");
    assert_eq!(body["error"]["retryable"], false);
}

#[test]
fn a_path_segment_that_cannot_be_an_instance_id_is_reported_as_absent() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified("GET", "/v1/sandboxes/not-an-instance", "");

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 404);
    assert!(facade.calls.is_empty());
    assert_eq!(body["error"]["code"], "machine_not_found");
}

#[test]
fn stopping_a_sandbox_reports_the_stopped_state() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified(
        "POST",
        &format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}/stop"),
        "",
    );

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 200);
    assert_eq!(facade.calls, vec![Call::Stop]);
    assert_eq!(body["operation"], "sandbox.stop");
    assert_eq!(body["result"]["state"], "stopped");
}

#[test]
fn destroying_a_sandbox_reports_the_destroyed_state() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified(
        "DELETE",
        &format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}"),
        "",
    );

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 200);
    assert_eq!(facade.calls, vec![Call::Destroy]);
    assert_eq!(body["operation"], "sandbox.destroy");
    assert_eq!(body["result"]["state"], "destroyed");
}

#[test]
fn a_state_conflict_answers_conflict_with_the_facade_conflict_code() {
    let mut facade = FakeFacade::new(Mode::Conflict);
    let request = identified(
        "DELETE",
        &format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}"),
        "",
    );

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 409);
    assert_eq!(body["error"]["code"], "state_conflict");
}
