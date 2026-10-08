use std::time::Duration;

use super::{
    Timing,
    params::{CreateParams, ExecParams},
    routing::{Forward, Route, route},
    timing::LaunchPhases,
};
use crate::http::request::Method;
use crate::runner::backend::CallTiming;

/// The retained launch receipt the fixtures carry, milestones and all.
const RECEIPT: &str = include_str!("../../../tests/fixtures/receipt.json");

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
    assert_eq!(
        route(&http::Method::PATCH, &format!("/api/v1/sandboxes/{id}")),
        Route::Extend(id)
    );
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
fn server_timing_names_every_segment_of_a_create() {
    let timing = Timing {
        auth: Duration::from_micros(12),
        call: CallTiming {
            pool: Duration::from_micros(1_500),
            exec: Duration::from_millis(21),
        },
        prep: Duration::from_micros(340),
        launch: LaunchPhases {
            resolve: Duration::from_micros(101),
            admit: Duration::from_micros(4_897),
            assign: Duration::from_millis(305),
            ready: Duration::from_millis(340),
            commit: Duration::from_millis(2),
        },
        finish: Duration::from_micros(120),
    };

    let header = timing.header();

    // The three segments the header always carried keep their names and order, so
    // a client reading `exec` today is unaffected.
    assert!(
        header.starts_with("auth;dur=0.012,pool;dur=1.500,exec;dur=21.000,"),
        "{header}"
    );
    for (name, expected) in [
        ("prep", "0.340"),
        ("resolve", "0.101"),
        ("admit", "4.897"),
        ("assign", "305.000"),
        ("ready", "340.000"),
        ("commit", "2.000"),
        ("finish", "0.120"),
    ] {
        assert!(
            header.contains(&format!("{name};dur={expected}")),
            "{name} missing from {header}"
        );
    }
    assert_eq!(header.split(',').count(), 10, "{header}");
}

#[test]
fn the_launch_segments_are_read_from_the_receipt_milestones() {
    let receipt: soma::ExecutionReceipt =
        serde_json::from_str(RECEIPT).expect("the retained receipt is valid");
    // The fixture's own milestones: accepted 0 ns, workload_resolved 101 ns,
    // admitted 4_998 ns, machine_launched 305_978_116 ns, ready 646_002_667 ns.
    let phases = LaunchPhases::from_receipt(&receipt, Duration::from_millis(900));

    assert_eq!(phases.resolve, Duration::from_nanos(101));
    assert_eq!(phases.admit, Duration::from_nanos(4_897));
    assert_eq!(phases.assign, Duration::from_nanos(305_973_118));
    assert_eq!(phases.ready, Duration::from_nanos(340_024_551));
    // `commit` is the remainder of the facade call after `ready`, so the segments
    // plus it reconstruct exactly the call the `exec` segment reports.
    assert_eq!(
        phases.resolve + phases.admit + phases.assign + phases.ready + phases.commit,
        Duration::from_millis(900)
    );
}

#[test]
fn a_ready_milestone_past_the_call_leaves_commit_at_zero() {
    // `commit` is derived by subtraction against the facade call's own duration,
    // which is a different clock from the milestone's; a disagreement must not
    // produce a segment larger than the call it belongs to.
    let receipt: soma::ExecutionReceipt =
        serde_json::from_str(RECEIPT).expect("the retained receipt is valid");
    let phases = LaunchPhases::from_receipt(&receipt, Duration::from_millis(1));

    assert_eq!(phases.commit, Duration::ZERO);
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
        CreateParams::parse(br#"{"timeout":0}"#, 3_600, &shape())
            .expect("0 is no idle timeout")
            .timeout_seconds,
        0
    );
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
        r#"{"timeout_sec":86401}"#,
        r#"{"timeout":-1}"#,
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

#[test]
fn a_create_field_the_platform_does_not_define_is_refused_by_name() {
    let error = CreateParams::parse(br#"{"size":"xs","runtime_profil":"soma"}"#, 3_600, &shape())
        .expect_err("a typo must be refused");
    assert_eq!(error.status, 400);
    assert_eq!(error.code, "INVALID_PARAM");
    assert!(
        error.message.contains("runtime_profil"),
        "the answer names the field: {}",
        error.message
    );
    assert_eq!(
        error.details,
        Some(serde_json::json!({"field": "runtime_profil"}))
    );

    for body in [
        r#"{"listn":"0.0.0.0:443"}"#,
        r#"{"snapshotid":"s"}"#,
        r#"{"metadata":"x","typo":1}"#,
    ] {
        assert_eq!(
            CreateParams::parse(body.as_bytes(), 3_600, &shape()).map_err(|error| error.code),
            Err("INVALID_PARAM"),
            "{body} must be refused"
        );
    }
}

#[test]
fn every_field_the_platform_defines_is_still_accepted() {
    for body in [
        r#"{"project_id":"p-1","timeout_sec":60,"region":"us","tags":{},"metadata":{},"disk_size_mb":4096,"idle_timeout_sec":0,"workspace_id":null,"agent_runtime_profile_id":null,"revision":null,"workdir":"/workspace"}"#,
        r#"{"size":"xs","wait":true,"response_format":"compact","persistent":false,"runtime_profile":"soma","timeout":0,"cpu_count":1,"memory_mb":512,"auto_start":false}"#,
    ] {
        assert!(
            CreateParams::parse(body.as_bytes(), 3_600, &shape()).is_ok(),
            "{body} must be accepted"
        );
    }
}

#[test]
fn a_create_carrying_a_working_directory_is_refused() {
    // Contract C2 lists a create with `cwd` among the requests the runner does not serve.
    assert_eq!(
        CreateParams::parse(br#"{"cwd":"/workspace"}"#, 3_600, &shape())
            .map_err(|error| error.code),
        Err("RUNNER_UNSUPPORTED_REQUEST")
    );
    assert!(CreateParams::parse(br#"{"cwd":null}"#, 3_600, &shape()).is_ok());
}
