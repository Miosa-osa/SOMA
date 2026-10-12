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

#[test]
fn a_command_over_the_size_bounds_is_refused_by_name_and_is_not_retryable() {
    let limit = soma::DirectCommand::MAX_ARGUMENT_BYTES;
    for body in [
        // One argument one byte over.
        format!(
            r#"{{"executable":"/bin/sh","arguments":["-lc","{}"]}}"#,
            "a".repeat(limit + 1)
        ),
        // Every argument within its bound but the whole command beyond the record.
        format!(
            r#"{{"executable":"/bin/sh","arguments":[{}]}}"#,
            vec![format!(r#""{}""#, "b".repeat(limit)); 17].join(",")
        ),
    ] {
        let mut facade = FakeFacade::new(Mode::Succeed);
        let (status, answer) = call(&mut facade, &identified("POST", &commands_path(), &body));

        assert_eq!(status, 413, "{answer}");
        assert!(facade.calls.is_empty(), "no machine may be asked");
        assert_eq!(answer["error"]["code"], "command_too_large");
        assert_eq!(answer["error"]["retryable"], false);
        let message = answer["error"]["message"].as_str().expect("a message");
        assert!(
            message.contains(&limit.to_string())
                && message.contains(&soma::DirectCommand::MAX_AGGREGATE_BYTES.to_string()),
            "the message must state the limits the runtime enforces: {message}"
        );
    }
}

#[test]
fn a_command_exactly_at_the_bound_is_admitted() {
    let limit = soma::DirectCommand::MAX_ARGUMENT_BYTES;
    let mut facade = FakeFacade::new(Mode::Succeed);
    let body = format!(
        r#"{{"executable":"/bin/sh","arguments":["-lc","{}"]}}"#,
        "a".repeat(limit)
    );
    let (status, _) = call(&mut facade, &identified("POST", &commands_path(), &body));
    assert_eq!(status, 200);
}
