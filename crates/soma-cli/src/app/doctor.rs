use std::path::PathBuf;

use soma_local::{LocalFailureKind, probe_backend};

use crate::{
    cli::BackendSelection,
    exit::ProcessExit,
    model::{DoctorReport, DoctorStatus, Response, ResultBody},
};

use super::Execution;

pub(super) fn doctor(
    selection: BackendSelection,
    runtime: Option<PathBuf>,
    strict: bool,
) -> Execution {
    let report = match probe_backend(selection.into(), runtime) {
        Ok(probe) => DoctorReport {
            backend: backend_name(selection),
            status: DoctorStatus::ProbePassed,
            supported_target: true,
            runtime_ready: true,
            production_ready: probe.production_ready(),
            runtime_version: Some(probe.runtime_version().to_owned()),
            reason: "backend_probe_passed",
        },
        Err(failure) => DoctorReport {
            backend: backend_name(selection),
            status: if failure.kind() == LocalFailureKind::UnsupportedTarget {
                DoctorStatus::Unsupported
            } else {
                DoctorStatus::ProbeFailed
            },
            supported_target: failure.kind() != LocalFailureKind::UnsupportedTarget,
            runtime_ready: false,
            production_ready: false,
            runtime_version: None,
            reason: local_failure_reason(failure.kind()),
        },
    };
    let exit = if strict && !report.passed() {
        ProcessExit::DoctorStrict
    } else {
        ProcessExit::Success
    };
    Execution {
        response: Response::success("doctor", ResultBody::Doctor(report)),
        exit,
    }
}

const fn backend_name(selection: BackendSelection) -> &'static str {
    match selection {
        BackendSelection::Auto => "auto",
        BackendSelection::Macos => "macos",
        BackendSelection::Kvm => "kvm",
    }
}

const fn local_failure_reason(kind: LocalFailureKind) -> &'static str {
    match kind {
        LocalFailureKind::InvalidConfiguration => "invalid_configuration",
        LocalFailureKind::UnsupportedTarget => "unsupported_target",
        LocalFailureKind::BackendUnavailable => "backend_unavailable",
        LocalFailureKind::StateStore(_) => "state_store_failure",
    }
}
