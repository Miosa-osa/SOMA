use std::time::Duration;

use super::{
    Timing,
    params::{CreateParams, ExecParams},
    routing::{Forward, Route, route},
};
use crate::http::request::Method;
use crate::runner::backend::CallTiming;
use crate::runner::config::{LargeShape, LaunchConfig};

pub(super) fn shape() -> soma::MachineShape {
    soma::MachineShape::new(1, 512, 2_048).expect("valid shape")
}

/// A launch configuration that serves exactly the one shape, as every runner did before the
/// large size existed.
pub(super) fn launch() -> LaunchConfig {
    LaunchConfig {
        image: "docker.io/library/node:22".into(),
        shape: shape(),
        large: None,
        template_id: "miosa-sandbox-soma".into(),
        default_timeout_seconds: 300,
    }
}

/// The same runner, with the large shape configured.
pub(super) fn launch_serving_large() -> LaunchConfig {
    LaunchConfig {
        large: Some(LargeShape {
            vcpu_count: 8,
            memory_mib: 16 * 1024,
            storage_mib: 20 * 1024,
            public_egress: true,
        }),
        ..launch()
    }
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
        &launch(),
    )
    .expect("accepted");

    assert_eq!(parsed.project_id.as_deref(), Some("p-1"));
    assert_eq!(parsed.timeout_seconds, 3_600);
    assert_eq!(
        CreateParams::parse(br#"{"timeout":0}"#, 3_600, &launch())
            .expect("0 is no idle timeout")
            .timeout_seconds,
        0
    );
    assert_eq!(
        CreateParams::parse(b"", 3_600, &launch())
            .expect("empty body")
            .timeout_seconds,
        3_600
    );
    assert_eq!(
        CreateParams::parse(
            br#"{"timeout_sec":60,"cpu_count":1,"memory_mb":512}"#,
            3_600,
            &launch()
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
            CreateParams::parse(body.as_bytes(), 3_600, &launch()).is_err(),
            "{body} must be refused"
        );
    }
    assert!(
        CreateParams::parse(
            br#"{"env":{},"name":"","snapshot_id":null}"#,
            3_600,
            &launch()
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
    let error = CreateParams::parse(
        br#"{"size":"xs","runtime_profil":"soma"}"#,
        3_600,
        &launch(),
    )
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
            CreateParams::parse(body.as_bytes(), 3_600, &launch()).map_err(|error| error.code),
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
            CreateParams::parse(body.as_bytes(), 3_600, &launch()).is_ok(),
            "{body} must be accepted"
        );
    }
}

#[test]
fn a_create_carrying_a_working_directory_is_refused() {
    // Contract C2 lists a create with `cwd` among the requests the runner does not serve.
    assert_eq!(
        CreateParams::parse(br#"{"cwd":"/workspace"}"#, 3_600, &launch())
            .map_err(|error| error.code),
        Err("RUNNER_UNSUPPORTED_REQUEST")
    );
    assert!(CreateParams::parse(br#"{"cwd":null}"#, 3_600, &launch()).is_ok());
}

#[test]
fn a_refused_command_is_not_reported_as_an_agent_outage() {
    use soma::{BackendFailureKind, ManagedFailure};

    use super::command::failure_error;

    // Each of these is a property of the request rather than of the machine, so each answers its
    // own 4xx/5xx instead of sending the caller back to retry something that cannot succeed.
    for (kind, status, code) in [
        (
            BackendFailureKind::WorkloadRejected,
            400,
            "WORKLOAD_REJECTED",
        ),
        (
            BackendFailureKind::ResourceConflict,
            409,
            "RESOURCE_CONFLICT",
        ),
        (BackendFailureKind::Unsupported, 501, "BACKEND_UNSUPPORTED"),
    ] {
        let error = failure_error(&ManagedFailure::Backend(kind));
        assert_eq!((error.status, error.code), (status, code), "{kind:?}");
        assert!(!error.retryable, "{kind:?} must not invite a retry");
    }

    // A machine that actually went missing is still an outage.
    let lost = failure_error(&ManagedFailure::Backend(BackendFailureKind::GuestFailure));
    assert_eq!((lost.status, lost.code), (502, "AGENT_UNAVAILABLE"));
    assert!(lost.retryable);
}
