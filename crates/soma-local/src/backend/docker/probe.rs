use std::time::Duration;

use soma::BackendKind;

use super::command::COMMAND;
use super::process;
use crate::backend::{BackendProbe, LocalFailure, LocalFailureKind};

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub(in crate::backend) fn is_available() -> bool {
    process::run(
        COMMAND,
        &[
            "info".to_owned(),
            "--format".to_owned(),
            "{{.ServerVersion}}".to_owned(),
        ],
        Duration::from_secs(5),
        128,
    )
    .is_ok_and(|result| result.status.is_some_and(|status| status.success()))
}

pub(in crate::backend) fn probe() -> Result<BackendProbe, LocalFailure> {
    let result = process::run(
        COMMAND,
        &[
            "version".to_owned(),
            "--format".to_owned(),
            "{{.Server.Version}}".to_owned(),
        ],
        Duration::from_secs(5),
        128,
    )
    .map_err(|_| LocalFailure::new(LocalFailureKind::BackendUnavailable))?;
    if !result.status.is_some_and(|status| status.success()) {
        return Err(LocalFailure::new(LocalFailureKind::BackendUnavailable));
    }
    let version = String::from_utf8_lossy(&result.stdout).trim().to_owned();
    Ok(BackendProbe {
        backend: BackendKind::DockerContainer,
        runtime_version: format!("docker-{version}"),
        production_ready: false,
    })
}
