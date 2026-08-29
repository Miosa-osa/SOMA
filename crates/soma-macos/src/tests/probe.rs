use crate::{
    BackendClass, BackendError, CommandFailureReason, IsolationKind, Operation,
    SUPPORTED_CONTAINER_VERSION_REQUIREMENT,
};

use super::fixtures::{backend, strings, success};

const VERSION_JSON: &str = r#"[
  {"appName":"container","buildType":"release","commit":"abc123","version":"1.3.0"},
  {"appName":"container-apiserver","buildType":"release","commit":"def456","version":"container-apiserver version 1.3.0 (build: release, commit: def456)"}
]"#;

#[test]
fn probe_requires_supported_version_and_running_service() {
    let (backend, runner) = backend([
        Ok(success(VERSION_JSON.as_bytes())),
        Ok(success(
            br#"{"status":"running","appRoot":"/private/runtime"}"#,
        )),
    ]);

    let report = backend.probe().expect("runtime is available");

    assert_eq!(report.backend_class(), BackendClass::DevelopmentOnly);
    assert_eq!(
        report.isolation(),
        IsolationKind::VirtualMachinePerOciContainer
    );
    assert!(report.runtime_ready());
    assert_eq!(report.cli().version(), "1.3.0");
    assert_eq!(
        report.api_server().expect("server component").name(),
        "container-apiserver"
    );
    let calls = runner.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].program, "/opt/apple/bin/container");
    assert_eq!(
        calls[0].arguments,
        strings(&["system", "version", "--format", "json"])
    );
    assert_eq!(
        calls[1].arguments,
        strings(&["system", "status", "--format", "json"])
    );
}

#[test]
fn probe_rejects_a_stopped_runtime_even_when_the_command_exits_zero() {
    let (backend, _) = backend([
        Ok(success(VERSION_JSON.as_bytes())),
        Ok(success(br#"{"status":"stopped"}"#)),
    ]);

    let failure = backend
        .probe()
        .expect_err("stopped service is not capability evidence");

    assert_eq!(
        failure,
        BackendError::Command {
            failure: crate::CommandFailure::new(
                Operation::ProbeStatus,
                CommandFailureReason::RuntimeNotRunning,
            ),
        }
    );
}

#[test]
fn probe_rejects_malformed_status_json() {
    let (backend, _) = backend([
        Ok(success(VERSION_JSON.as_bytes())),
        Ok(success(br#"{"state":"running"}"#)),
    ]);

    let failure = backend
        .probe()
        .expect_err("missing status is not readiness evidence");

    assert_eq!(
        failure,
        BackendError::Command {
            failure: crate::CommandFailure::new(
                Operation::ProbeStatus,
                CommandFailureReason::InvalidJson,
            ),
        }
    );
}

#[test]
fn probe_fails_closed_on_a_future_unverified_minor_version() {
    let future = VERSION_JSON.replace("1.3.0", "1.4.0");
    let (backend, runner) = backend([Ok(success(future.into_bytes()))]);

    let failure = backend
        .probe()
        .expect_err("unverified CLI contracts fail closed");

    assert_eq!(
        failure,
        BackendError::UnsupportedVersion {
            found: "1.4.0".to_owned(),
            supported: SUPPORTED_CONTAINER_VERSION_REQUIREMENT,
        }
    );
    assert_eq!(runner.calls().len(), 1);
}
