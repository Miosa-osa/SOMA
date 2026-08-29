use std::ffi::OsString;

use semver::{Version, VersionReq};
use serde::Deserialize;

use crate::{
    BackendError, CapabilityReport, CommandFailure, CommandFailureReason, ComponentVersion,
    Operation, SUPPORTED_CONTAINER_VERSION_REQUIREMENT,
};

use super::MacOsBackend;

const PROBE_TIMEOUT_MILLIS: u64 = 5_000;
const PROBE_OUTPUT_BYTES: u64 = 65_536;

impl MacOsBackend {
    /// Proves that the supported Apple CLI and its runtime service are available.
    ///
    /// # Errors
    ///
    /// Returns a typed error for an unsupported host or version, an unavailable process, a stopped
    /// runtime service, or malformed version output.
    pub fn probe(&self) -> Result<CapabilityReport, BackendError> {
        self.ensure_host()?;
        let version_output = self
            .commands
            .execute(
                Operation::ProbeVersion,
                strings(["system", "version", "--format", "json"]),
                PROBE_TIMEOUT_MILLIS,
                PROBE_OUTPUT_BYTES,
            )
            .map_err(BackendError::command)?;
        require_success(Operation::ProbeVersion, version_output.status())?;

        let components =
            serde_json::from_slice::<Vec<RawComponentVersion>>(version_output.stdout())
                .map_err(|_| invalid_response(Operation::ProbeVersion))?;
        let cli = components
            .iter()
            .find(|component| component.app_name == "container")
            .ok_or_else(|| missing_component(Operation::ProbeVersion))?;
        ensure_supported_version(&cli.version)?;

        let status_output = self
            .commands
            .execute(
                Operation::ProbeStatus,
                strings(["system", "status", "--format", "json"]),
                PROBE_TIMEOUT_MILLIS,
                PROBE_OUTPUT_BYTES,
            )
            .map_err(BackendError::command)?;
        require_success(Operation::ProbeStatus, status_output.status())?;
        let runtime_status = serde_json::from_slice::<RawRuntimeStatus>(status_output.stdout())
            .map_err(|_| invalid_response(Operation::ProbeStatus))?;
        if runtime_status.status != "running" {
            return Err(BackendError::command(CommandFailure::new(
                Operation::ProbeStatus,
                CommandFailureReason::RuntimeNotRunning,
            )));
        }

        let cli = cli.clone().into_public();
        let api_server = components
            .into_iter()
            .find(|component| component.app_name == "container-apiserver")
            .map(RawComponentVersion::into_public);
        Ok(CapabilityReport::new(cli, api_server))
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawComponentVersion {
    app_name: String,
    build_type: String,
    commit: String,
    version: String,
}

#[derive(Deserialize)]
struct RawRuntimeStatus {
    status: String,
}

impl RawComponentVersion {
    fn into_public(self) -> ComponentVersion {
        ComponentVersion::new(self.app_name, self.version, self.build_type, self.commit)
    }
}

fn ensure_supported_version(found: &str) -> Result<(), BackendError> {
    let found_version =
        Version::parse(found).map_err(|_| invalid_response(Operation::ProbeVersion))?;
    let supported = VersionReq::parse(SUPPORTED_CONTAINER_VERSION_REQUIREMENT)
        .expect("the compile-time container version requirement is valid");
    if !supported.matches(&found_version) {
        return Err(BackendError::UnsupportedVersion {
            found: found.to_owned(),
            supported: SUPPORTED_CONTAINER_VERSION_REQUIREMENT,
        });
    }
    Ok(())
}

fn require_success(
    operation: Operation,
    status: crate::ExecutionStatus,
) -> Result<(), BackendError> {
    if status.is_success() {
        Ok(())
    } else {
        Err(BackendError::command(CommandFailure::new(
            operation,
            CommandFailureReason::Status(status),
        )))
    }
}

fn invalid_response(operation: Operation) -> BackendError {
    BackendError::command(CommandFailure::new(
        operation,
        CommandFailureReason::InvalidJson,
    ))
}

fn missing_component(operation: Operation) -> BackendError {
    BackendError::command(CommandFailure::new(
        operation,
        CommandFailureReason::MissingVersionComponent,
    ))
}

fn strings<const N: usize>(values: [&str; N]) -> Vec<OsString> {
    values.into_iter().map(OsString::from).collect()
}
