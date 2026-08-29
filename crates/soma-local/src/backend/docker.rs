mod process;

use std::time::Duration;

use serde::Deserialize;
use soma::{
    BackendFailure, BackendFailureKind, BackendKind, CleanupEvidence, CleanupMethod,
    CleanupObservation, CleanupRequest, CleanupTimes, CommandObservation, CommandStatus,
    CommandTimes, DigestBinding, DnsPolicy, EffectiveNetwork, EffectiveShape, EgressPolicy,
    ExecutionRequest, InspectionObservation, InspectionRequest, IsolationClass, LaunchObservation,
    LaunchRequest, LaunchTimes, MachineState, NetworkAttachment, Observation,
    ObservationUnavailable, OciDigest, OciPlatform, PortActivationClass, PreparationClass,
    ResolutionObservation, ResolutionRequest, WorkloadIdentity,
};

use super::{BackendProbe, LocalFailure, LocalFailureKind, clock::OperationClocks};

const COMMAND: &str = "docker";
const CONTROL_TIMEOUT: Duration = Duration::from_secs(60);
const CONTROL_OUTPUT_LIMIT: usize = 1_048_576;

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

pub(crate) struct DockerBackend {
    already_cleaned: std::collections::BTreeSet<String>,
    clocks: OperationClocks,
}

struct DockerPreparedWorkload {
    image: String,
    identity: WorkloadIdentity,
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

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub(super) fn is_available() -> bool {
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

pub(super) fn probe() -> Result<BackendProbe, LocalFailure> {
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

impl DockerBackend {
    pub(super) fn open() -> Result<Self, LocalFailure> {
        probe()?;
        Ok(Self {
            already_cleaned: std::collections::BTreeSet::new(),
            clocks: OperationClocks::new(),
        })
    }

    pub(super) const fn kind() -> BackendKind {
        BackendKind::DockerContainer
    }

    pub(super) fn resolve(
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

    pub(super) fn launch(
        &mut self,
        request: &LaunchRequest<'_, Box<dyn std::any::Any + Send>>,
    ) -> Result<LaunchObservation, BackendFailure> {
        let operation = request.operation_id();
        let admitted = self.clocks.elapsed_ns(operation);
        let prepared = request
            .prepared()
            .downcast_ref::<DockerPreparedWorkload>()
            .ok_or_else(|| failure(operation, BackendFailureKind::WorkloadRejected))?;
        if prepared.identity != *request.workload() {
            return Err(failure(operation, BackendFailureKind::WorkloadRejected));
        }
        let name = container_name(request.instance_id().as_str());
        let network = request.shape().capabilities().network_policy();
        let network_mode = match (network.egress(), network.dns(), network.published_ports()) {
            (EgressPolicy::Denied, DnsPolicy::Denied, []) => "none",
            (EgressPolicy::Unrestricted, DnsPolicy::System, []) => "bridge",
            _ => return Err(failure(operation, BackendFailureKind::Unsupported)),
        };
        let memory = format!("{}m", request.shape().memory_mib());
        let cpus = request.shape().vcpu_count().to_string();
        let args = vec![
            "create".to_owned(),
            "--name".to_owned(),
            name.clone(),
            "--label".to_owned(),
            format!("com.miosa.soma.instance={}", request.instance_id().as_str()),
            "--cpus".to_owned(),
            cpus,
            "--memory".to_owned(),
            memory,
            "--pids-limit".to_owned(),
            "256".to_owned(),
            "--read-only".to_owned(),
            "--cap-drop".to_owned(),
            "ALL".to_owned(),
            "--security-opt".to_owned(),
            "no-new-privileges".to_owned(),
            "--tmpfs".to_owned(),
            "/tmp:rw,nosuid,nodev,size=64m".to_owned(),
            "--network".to_owned(),
            network_mode.to_owned(),
            "--entrypoint".to_owned(),
            "/bin/sh".to_owned(),
            prepared.image.clone(),
            "-c".to_owned(),
            "while :; do sleep 3600; done".to_owned(),
        ];
        let created = command_owned(&args, CONTROL_TIMEOUT);
        if !created.status.is_some_and(|status| status.success()) {
            return Err(failure(operation, BackendFailureKind::GuestFailure));
        }
        let started = command(&["start", &name], CONTROL_TIMEOUT);
        if !started.status.is_some_and(|status| status.success()) {
            let _ = remove(&name);
            return Err(failure(operation, BackendFailureKind::GuestFailure));
        }
        let launched = self.clocks.elapsed_ns(operation);
        let ready = self.clocks.elapsed_ns(operation);
        let shape = EffectiveShape::new(
            Observation::Observed(request.shape().vcpu_count()),
            Observation::Observed(request.shape().memory_mib()),
            Observation::Unavailable(ObservationUnavailable::NotVerified),
        )
        .map_err(|_| failure(operation, BackendFailureKind::IsolationFailure))?;
        Ok(LaunchObservation::new(
            operation.clone(),
            request.instance_id().clone(),
            request.workload().clone(),
            BackendKind::DockerContainer,
            IsolationClass::LinuxContainer,
            PreparationClass::OnDemand,
            DigestBinding::ObservedOnly,
            shape,
            effective_network(network_mode),
            LaunchTimes::new(admitted, launched, ready),
        ))
    }

    pub(super) fn execute(&mut self, request: ExecutionRequest<'_>) -> CommandObservation {
        let operation = request.operation_id();
        let started = self.clocks.elapsed_ns(operation);
        let name = container_name(request.instance_id().as_str());
        let mut args = vec![
            "exec".to_owned(),
            name.clone(),
            request.command().executable().to_owned(),
        ];
        args.extend(request.command().arguments().iter().map(ToOwned::to_owned));
        let result = command_owned(&args, Duration::from_millis(request.limits().timeout_ms()));
        if result.timed_out || result.output_limited {
            let _ = remove(&name);
            self.already_cleaned
                .insert(request.instance_id().as_str().to_owned());
        }
        let status = if result.timed_out {
            CommandStatus::TimedOut
        } else if result.output_limited {
            CommandStatus::OutputLimitExceeded
        } else {
            CommandStatus::Exited {
                code: process::status_code(result.status),
            }
        };
        CommandObservation::new(
            operation.clone(),
            request.instance_id().clone(),
            status,
            soma::ObservedOutput::new(
                result.stdout.clone(),
                result.stdout.len() as u64,
                result.stderr.clone(),
                result.stderr.len() as u64,
            ),
            CommandTimes::new(started, self.clocks.elapsed_ns(operation)),
        )
    }

    pub(super) fn inspect(
        &mut self,
        request: InspectionRequest<'_>,
    ) -> Result<InspectionObservation, BackendFailure> {
        let name = container_name(request.instance_id().as_str());
        let result = command(
            &["inspect", "--format", "{{.State.Status}}", &name],
            CONTROL_TIMEOUT,
        );
        let state = match String::from_utf8_lossy(&result.stdout).trim() {
            "running" => MachineState::Ready,
            "created" | "stopped" | "exited" => MachineState::Stopping,
            _ => {
                return Err(failure(
                    request.operation_id(),
                    BackendFailureKind::GuestFailure,
                ));
            }
        };
        let mode = if request.shape().capabilities().network_policy().egress()
            == EgressPolicy::Unrestricted
        {
            "bridge"
        } else {
            "none"
        };
        Ok(InspectionObservation::observed(
            request,
            BackendKind::DockerContainer,
            state,
            effective_network(mode),
            self.clocks.elapsed_ns(request.operation_id()),
        ))
    }

    pub(super) fn cleanup(&mut self, request: CleanupRequest<'_>) -> CleanupObservation {
        let key = request.instance_id().as_str().to_owned();
        let started = self.clocks.elapsed_ns(request.operation_id());
        let complete = if self.already_cleaned.remove(&key) {
            true
        } else {
            remove(&container_name(&key))
        };
        let evidence = if complete {
            CleanupEvidence::complete_owned_machine().with_method(CleanupMethod::Forced)
        } else {
            CleanupEvidence::incomplete_owned_machine().with_method(CleanupMethod::Forced)
        };
        CleanupObservation::new(
            request.operation_id().clone(),
            request.instance_id().clone(),
            evidence,
            CleanupTimes::new(started, self.clocks.elapsed_ns(request.operation_id())),
        )
    }
}

fn command(args: &[&str], timeout: Duration) -> process::Result {
    command_owned(
        &args
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>(),
        timeout,
    )
}

fn command_owned(args: &[String], timeout: Duration) -> process::Result {
    process::run(COMMAND, args, timeout, CONTROL_OUTPUT_LIMIT).unwrap_or(process::Result {
        status: None,
        stdout: Vec::new(),
        stderr: Vec::new(),
        timed_out: false,
        output_limited: false,
    })
}

fn remove(name: &str) -> bool {
    command(&["rm", "--force", name], CONTROL_TIMEOUT)
        .status
        .is_some_and(|status| status.success())
}

fn container_name(instance: &str) -> String {
    format!("soma-{instance}")
}

fn failure(operation: &soma::OperationId, kind: BackendFailureKind) -> BackendFailure {
    let _ = operation;
    BackendFailure::new(kind, 1)
}

fn effective_network(mode: &str) -> EffectiveNetwork {
    let attached = mode != "none";
    EffectiveNetwork::new(
        Observation::Observed(if attached {
            NetworkAttachment::Attached
        } else {
            NetworkAttachment::Detached
        }),
        Observation::Observed(if attached {
            EgressPolicy::Unrestricted
        } else {
            EgressPolicy::Denied
        }),
        Observation::Observed(if attached {
            DnsPolicy::System
        } else {
            DnsPolicy::Denied
        }),
        Observation::Observed(Vec::new()),
        Observation::Observed(Vec::new()),
        Observation::Observed(PortActivationClass::NotApplicable),
    )
    .expect("Docker network observations are canonical")
}
