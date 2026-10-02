use std::time::Duration;

use super::{
    Timing,
    params::{CreateParams, ExecParams},
    routing::{Forward, Route, route},
};
use crate::http::request::Method;
use crate::runner::backend::CallTiming;

fn shape() -> soma::MachineShape {
    soma::MachineShape::new(1, 512, 2_048).expect("valid shape")
}

#[test]
fn routes_only_the_contract_paths() {
    let id = "3f2504e0-4f89-41d3-9a0c-0305e82c3301";
    assert_eq!(
        route(&http::Method::POST, "/api/v1/sandboxes"),
        Route::Create
    );
    assert_eq!(
        route(&http::Method::POST, &format!("/api/v1/sandboxes/{id}/exec")),
        Route::Exec(id)
    );
    assert_eq!(
        route(&http::Method::DELETE, &format!("/api/v1/sandboxes/{id}")),
        Route::Destroy(id)
    );
    assert_eq!(route(&http::Method::GET, "/healthz"), Route::Health);
    assert_eq!(route(&http::Method::GET, "/api/v1/sandboxes"), Route::List);
    for (method, suffix, internal) in [
        (http::Method::GET, "", Method::Get),
        (http::Method::POST, "/stop", Method::Post),
        (http::Method::POST, "/filesystem/read", Method::Post),
        (http::Method::POST, "/terminal/open", Method::Post),
    ] {
        assert_eq!(
            route(&method, &format!("/api/v1/sandboxes/{id}{suffix}")),
            Route::Forward(Forward {
                id,
                method: internal,
                suffix
            }),
            "{method} {suffix}"
        );
    }
    for (method, path) in [
        (http::Method::POST, "/api/v1/sandboxes/"),
        (http::Method::POST, "/api/v1/sandboxesx"),
        (
            http::Method::POST,
            &format!("/api/v1/sandboxes/{id}/commands") as &str,
        ),
        (
            http::Method::POST,
            &format!("/api/v1/sandboxes/{id}/filesystem/"),
        ),
        (http::Method::GET, &format!("/api/v1/sandboxes/{id}/stop")),
        (http::Method::POST, "/healthz"),
        (http::Method::GET, "/"),
    ] {
        assert_eq!(route(&method, path), Route::NotFound, "{method} {path}");
    }
}

#[test]
fn server_timing_reports_three_millisecond_segments() {
    let timing = Timing {
        auth: Duration::from_micros(12),
        call: CallTiming {
            pool: Duration::from_micros(1_500),
            exec: Duration::from_millis(21),
        },
    };

    assert_eq!(
        timing.header(),
        "auth;dur=0.012,pool;dur=1.500,exec;dur=21.000"
    );
}

#[test]
fn a_bare_create_is_accepted_with_defaults() {
    let parsed = CreateParams::parse(
        br#"{"size":"xs","wait":true,"response_format":"compact","persistent":false,"runtime_profile":"soma","project_id":"p-1","metadata":{"a":"b"}}"#,
        3_600,
        &shape(),
    )
    .expect("accepted");

    assert_eq!(parsed.project_id.as_deref(), Some("p-1"));
    assert_eq!(parsed.timeout_seconds, 3_600);
    assert_eq!(
        CreateParams::parse(b"", 3_600, &shape())
            .expect("empty body")
            .timeout_seconds,
        3_600
    );
    assert_eq!(
        CreateParams::parse(
            br#"{"timeout_sec":60,"cpu_count":1,"memory_mb":512}"#,
            3_600,
            &shape()
        )
        .expect("matching shape")
        .timeout_seconds,
        60
    );
}

#[test]
fn a_create_the_fast_lane_would_not_serve_is_refused() {
    for body in [
        r#"{"template_id":"node"}"#,
        r#"{"image":"python:3"}"#,
        r#"{"size":"m"}"#,
        r#"{"persistent":true}"#,
        r#"{"persistent":"1"}"#,
        r#"{"auto_start":true}"#,
        r#"{"runtime_profile":"firecracker"}"#,
        r#"{"env":{"A":"1"}}"#,
        r#"{"cpu_count":4}"#,
        r#"{"timeout_sec":0}"#,
        r#"{"timeout_sec":"60"}"#,
        r#"{"project_id":7}"#,
        "[1]",
        "not json",
    ] {
        assert!(
            CreateParams::parse(body.as_bytes(), 3_600, &shape()).is_err(),
            "{body} must be refused"
        );
    }
    assert!(
        CreateParams::parse(
            br#"{"env":{},"name":"","snapshot_id":null}"#,
            3_600,
            &shape()
        )
        .is_ok()
    );
}

#[test]
fn exec_parameters_follow_the_fast_lane() {
    assert_eq!(
        ExecParams::parse(br#"{"command":"node -v"}"#).expect("accepted"),
        ExecParams {
            command: "node -v".to_owned(),
            timeout_ms: 30_000
        }
    );
    assert_eq!(
        ExecParams::parse(br#"{"command":"ls","timeout":5,"cwd":null}"#)
            .expect("accepted")
            .timeout_ms,
        5_000
    );
    assert_eq!(
        ExecParams::parse(br#"{"command":""}"#).map_err(|error| error.code),
        Err("MISSING_PARAM")
    );
    assert_eq!(
        ExecParams::parse(b"").map_err(|error| error.code),
        Err("MISSING_PARAM")
    );
    assert_eq!(
        ExecParams::parse(br#"{"command":"ls","timeout":0}"#).map_err(|error| error.code),
        Err("INVALID_TIMEOUT")
    );
    assert_eq!(
        ExecParams::parse(br#"{"command":"ls","cwd":"/tmp"}"#).map_err(|error| error.code),
        Err("RUNNER_UNSUPPORTED_REQUEST")
    );
}
