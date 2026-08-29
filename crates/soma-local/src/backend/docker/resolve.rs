use serde::Deserialize;
use soma::{
    BackendFailure, BackendFailureKind, OciDigest, OciPlatform, ResolutionObservation,
    ResolutionRequest, WorkloadIdentity,
};

use super::command::{CONTROL_TIMEOUT, command};
use super::{DockerBackend, failure};

/// The only OCI architecture this host can execute through Docker without emulation.
///
/// Docker Desktop on Apple Silicon and Docker Engine on `x86_64` run their native Linux
/// architecture; any other image platform is rejected rather than silently emulated.
#[cfg(target_arch = "x86_64")]
const HOST_OCI_ARCHITECTURE: Option<&str> = Some("amd64");
#[cfg(target_arch = "aarch64")]
const HOST_OCI_ARCHITECTURE: Option<&str> = Some("arm64");
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
const HOST_OCI_ARCHITECTURE: Option<&str> = None;

pub(super) struct DockerPreparedWorkload {
    pub(super) image: String,
    pub(super) identity: WorkloadIdentity,
}

#[derive(Deserialize)]
struct ImageInspection {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Os")]
    os: String,
    #[serde(rename = "Architecture")]
    architecture: String,
    #[serde(rename = "Variant")]
    variant: Option<String>,
    #[serde(rename = "RepoDigests", default)]
    repo_digests: Vec<String>,
}

impl DockerBackend {
    pub(in crate::backend) fn resolve(
        &mut self,
        request: ResolutionRequest<'_>,
    ) -> Result<ResolutionObservation<Box<dyn std::any::Any + Send>>, BackendFailure> {
        let operation = request.operation_id();
        self.clocks.elapsed_ns(operation);
        let image = request.image().as_str().to_owned();
        let Some(architecture) = HOST_OCI_ARCHITECTURE else {
            return Err(failure(operation, BackendFailureKind::Unsupported));
        };
        let platform = format!("linux/{architecture}");
        let pull = command(&["pull", "--platform", &platform, &image], CONTROL_TIMEOUT);
        if !pull.status.is_some_and(|status| status.success()) {
            return Err(failure(operation, BackendFailureKind::Unavailable));
        }
        let inspected = command(
            &["image", "inspect", "--format", "{{json .}}", &image],
            CONTROL_TIMEOUT,
        );
        if !inspected.status.is_some_and(|status| status.success()) {
            return Err(failure(operation, BackendFailureKind::WorkloadRejected));
        }
        let record: ImageInspection = serde_json::from_slice(&inspected.stdout)
            .map_err(|_| failure(operation, BackendFailureKind::WorkloadRejected))?;
        if record.os != "linux" || record.architecture != architecture {
            return Err(failure(operation, BackendFailureKind::WorkloadRejected));
        }
        let digest = record
            .repo_digests
            .iter()
            .find_map(|value| value.split_once('@').map(|(_, digest)| digest))
            .unwrap_or(record.id.trim_start_matches("sha256:").trim());
        let manifest = OciDigest::parse(digest.to_owned())
            .map_err(|_| failure(operation, BackendFailureKind::WorkloadRejected))?;
        let platform = OciPlatform::new(
            "linux",
            architecture,
            record.variant.filter(|value| !value.is_empty()),
        )
        .map_err(|_| failure(operation, BackendFailureKind::WorkloadRejected))?;
        let identity = WorkloadIdentity::new(manifest, platform, None);
        let prepared = DockerPreparedWorkload {
            image,
            identity: identity.clone(),
        };
        Ok(ResolutionObservation::new(
            operation.clone(),
            request.source_fingerprint().clone(),
            identity,
            Box::new(prepared),
            self.clocks.elapsed_ns(operation),
        ))
    }
}
