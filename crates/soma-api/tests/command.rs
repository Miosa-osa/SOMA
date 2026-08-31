#[allow(
    dead_code,
    reason = "each integration test uses a subset of the shared fixtures"
)]
mod support;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use support::{Call, FIXTURE_INSTANCE_ID, FakeFacade, Mode, call, identified};

fn commands_path() -> String {
    format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}/commands")
}

#[test]
fn running_a_command_returns_base64_output_and_the_execution_receipt() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified(
        "POST",
        &commands_path(),
        r#"{"executable":"/usr/local/bin/node","arguments":["--version"]}"#,
    );

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 200);
    assert_eq!(facade.calls, vec![Call::Execute]);
    assert_eq!(body["operation"], "sandbox.command");
    assert_eq!(body["result"]["execution"]["exited"]["code"], 0);
    assert_eq!(body["result"]["stdout"]["encoding"], "base64");
    assert_eq!(body["result"]["stdout"]["byte_length"], 9);
    assert_eq!(
        body["result"]["stdout"]["data"],
        STANDARD.encode("v22.23.2\n")
    );
    assert_eq!(body["result"]["stderr"]["byte_length"], 0);
    assert!(body["receipt"].is_object());
}

#[test]
fn running_a_command_accepts_explicit_execution_limits() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified(
        "POST",
        &commands_path(),
        r#"{"executable":"/bin/true","limits":{"timeout_ms":5000,"max_output_bytes":4096}}"#,
    );

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 200);
    assert_eq!(body["status"], "ok");
}

#[test]
fn a_relative_executable_is_refused_before_the_engine_is_touched() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified("POST", &commands_path(), r#"{"executable":"node"}"#);

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 400);
    assert!(facade.calls.is_empty());
    assert_eq!(body["error"]["code"], "invalid_input");
}

#[test]
fn limits_outside_the_facade_range_are_refused_before_the_engine_is_touched() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified(
        "POST",
        &commands_path(),
        r#"{"executable":"/bin/true","limits":{"timeout_ms":0,"max_output_bytes":4096}}"#,
    );

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 400);
    assert!(facade.calls.is_empty());
    assert_eq!(body["error"]["code"], "invalid_input");
}

#[test]
fn a_command_body_is_required_rather_than_defaulted() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified("POST", &commands_path(), "");

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 400);
    assert!(facade.calls.is_empty());
    assert_eq!(body["error"]["code"], "invalid_input");
}

#[test]
fn a_command_against_an_absent_sandbox_reports_the_facade_not_found_code() {
    let mut facade = FakeFacade::new(Mode::NotFound);
    let request = identified(
        "POST",
        &commands_path(),
        r#"{"executable":"/bin/true","arguments":[]}"#,
    );

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 404);
    assert_eq!(facade.calls, vec![Call::Execute]);
    assert_eq!(body["error"]["code"], "machine_not_found");
}
