mod clock;
mod docker;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod kvm;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod macos;

use std::{any::Any, path::PathBuf};

use soma::{
    Backend, BackendFailure, BackendKind, CleanupObservation, CleanupRequest, CommandObservation,
    ExecutionRequest, InspectionObservation, InspectionRequest, LaunchObservation, LaunchRequest,
    ResolutionObservation, ResolutionRequest,
};

use crate::{LocalFailure, LocalFailureKind};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) use kvm::host_machine;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BackendSelection {
    #[default]
    Auto,
    Macos,
    Docker,
    Kvm,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendProbe {
    backend: BackendKind,
    runtime_version: String,
    production_ready: bool,
}

impl BackendProbe {
    #[must_use]
    pub const fn backend(&self) -> BackendKind {
        self.backend
    }

    #[must_use]
    pub fn runtime_version(&self) -> &str {
        &self.runtime_version
    }

    #[must_use]
    pub const fn production_ready(&self) -> bool {
        self.production_ready
    }
}

/// Probes one selected local backend without starting a lifecycle operation.
///
/// # Errors
///
/// Returns a typed failure when the target is unsupported or its real runtime probe fails.
pub fn probe_backend(
    selection: BackendSelection,
    explicit_runtime: Option<PathBuf>,
) -> Result<BackendProbe, LocalFailure> {
    let selection = resolve_selection(selection)?;
    match selection {
        BackendSelection::Macos => probe_macos(explicit_runtime),
        BackendSelection::Docker => docker::probe(),
        BackendSelection::Kvm => probe_kvm(),
        BackendSelection::Auto => unreachable!("auto selection is resolved before probing"),
    }
}

/// Whether a Machine one backend launches is still addressable once the launching process is
/// gone.
///
/// This is the difference between an instance identity a later command can use and one that
/// names a Machine which died with the process that reported it. A surface that hands an
/// identity back has to know which it is holding, because reporting a launch as ready without
/// it is reporting a success no second process can act on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MachineHosting {
    /// The Machine and its guest session are resident in the launching process. No backend
    /// answers this today; the surfaces that refuse on it are a guard for the next one.
    LaunchingProcess,
    /// The Machine is hosted outside the launching process and outlives it.
    OutlivesProcess,
}

/// How long a Machine launched on `backend` remains reachable.
#[must_use]
pub const fn machine_hosting(backend: BackendKind) -> MachineHosting {
    match backend {
        // Every backend keeps the machine somewhere the launching process is not, by two
        // routes. Docker and Apple `container` each register the machine with a runtime service
        // this process neither starts nor owns, under a name derived from the Instance, and
        // re-find it by that name on every later call; a KVM managed Launch starts a host
        // process answering on a socket named by the Instance. Either way a later command in a
        // later process reaches the machine by identity alone.
        BackendKind::DockerContainer
        | BackendKind::MacosVirtualization
        | BackendKind::Remote
        | BackendKind::LinuxKvm => MachineHosting::OutlivesProcess,
    }
}

/// Where machines that outlive their launching process are addressed.
///
/// It sits under the same durable state root the lifecycle records are in, because an Instance
/// whose durable record says it is active and whose host is somewhere else would be two truths a
/// caller had to reconcile.
pub(crate) fn machine_host_directory(state_root: &std::path::Path) -> PathBuf {
    state_root.join("machines")
}

pub(crate) enum LocalBackend {
    Docker(docker::DockerBackend),
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    Macos(macos::MacBackend),
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    Kvm(Box<kvm::KvmBackend>),
}

type PreparedWorkload = Box<dyn Any + Send>;

impl LocalBackend {
    /// Opens one local backend.
    ///
    /// `host_directory` is where machines that must outlive this process are addressed. Only a
    /// caller performing the managed Machine lifecycle supplies one; a one-shot run does not,
    /// and keeps its machine in its own process.
    pub(crate) fn open(
        selection: BackendSelection,
        explicit_runtime: Option<PathBuf>,
        host_directory: Option<PathBuf>,
    ) -> Result<(Self, BackendSelection), LocalFailure> {
        let resolved = resolve_selection(selection)?;
        match resolved {
            BackendSelection::Docker => {
                drop(host_directory);
                docker::DockerBackend::open()
                    .map(|backend| (Self::Docker(backend), resolved))
                    .map_err(|_| LocalFailure::new(LocalFailureKind::BackendUnavailable))
            }
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            BackendSelection::Macos => {
                drop(host_directory);
                macos::MacBackend::open(explicit_runtime)
                    .map(|backend| (Self::Macos(backend), resolved))
                    .map_err(|_| LocalFailure::new(LocalFailureKind::BackendUnavailable))
            }
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            BackendSelection::Kvm => {
                drop(explicit_runtime);
                kvm::KvmBackend::open(host_directory)
                    .map(|backend| (Self::Kvm(Box::new(backend)), resolved))
            }
            _ => {
                drop(explicit_runtime);
                drop(host_directory);
                Err(LocalFailure::new(LocalFailureKind::UnsupportedTarget))
            }
        }
    }
}

impl Backend for LocalBackend {
    type PreparedWorkload = PreparedWorkload;

    fn kind(&self) -> BackendKind {
        match self {
            Self::Docker(_) => docker::DockerBackend::kind(),
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            Self::Macos(_) => macos::MacBackend::kind(),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            Self::Kvm(_) => kvm::KvmBackend::kind(),
        }
    }

    fn resolve(
        &mut self,
        request: ResolutionRequest<'_>,
    ) -> Result<ResolutionObservation<Self::PreparedWorkload>, BackendFailure> {
        match self {
            Self::Docker(backend) => backend.resolve(request),
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            Self::Macos(backend) => backend.resolve_box(request),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            Self::Kvm(backend) => backend.resolve(request),
        }
    }

    fn launch(
        &mut self,
        request: LaunchRequest<'_, Self::PreparedWorkload>,
    ) -> Result<LaunchObservation, BackendFailure> {
        match self {
            Self::Docker(backend) => backend.launch(&request),
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            Self::Macos(backend) => backend.launch_box(&request),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            Self::Kvm(backend) => backend.launch(&request),
        }
    }

    fn execute(
        &mut self,
        request: ExecutionRequest<'_>,
    ) -> Result<CommandObservation, BackendFailure> {
        match self {
            Self::Docker(backend) => Ok(backend.execute(request)),
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            Self::Macos(backend) => backend.execute(request),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            Self::Kvm(backend) => backend.execute(request),
        }
    }

    fn inspect(
        &mut self,
        request: InspectionRequest<'_>,
    ) -> Result<InspectionObservation, BackendFailure> {
        match self {
            Self::Docker(backend) => backend.inspect(request),
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            Self::Macos(backend) => backend.inspect(request),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            Self::Kvm(backend) => backend.inspect(request),
        }
    }

    fn cleanup(
        &mut self,
        request: CleanupRequest<'_>,
    ) -> Result<CleanupObservation, BackendFailure> {
        match self {
            Self::Docker(backend) => Ok(backend.cleanup(request)),
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            Self::Macos(backend) => backend.cleanup(request),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            Self::Kvm(backend) => backend.cleanup(request),
        }
    }
}

fn resolve_selection(selection: BackendSelection) -> Result<BackendSelection, LocalFailure> {
    if selection != BackendSelection::Auto {
        return Ok(selection);
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        if docker::is_available() {
            return Ok(BackendSelection::Docker);
        }
        return Ok(BackendSelection::Macos);
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Ok(BackendSelection::Kvm);
    }
    #[allow(unreachable_code)]
    Err(LocalFailure::new(LocalFailureKind::UnsupportedTarget))
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn probe_macos(explicit_runtime: Option<PathBuf>) -> Result<BackendProbe, LocalFailure> {
    let backend = macos::MacBackend::open(explicit_runtime)
        .map_err(|_| LocalFailure::new(LocalFailureKind::BackendUnavailable))?;
    Ok(BackendProbe {
        backend: macos::MacBackend::kind(),
        runtime_version: backend.runtime_version().to_owned(),
        production_ready: false,
    })
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn probe_macos(_explicit_runtime: Option<PathBuf>) -> Result<BackendProbe, LocalFailure> {
    Err(LocalFailure::new(LocalFailureKind::UnsupportedTarget))
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn probe_kvm() -> Result<BackendProbe, LocalFailure> {
    let probe =
        soma_kvm::probe().map_err(|_| LocalFailure::new(LocalFailureKind::BackendUnavailable))?;
    Ok(BackendProbe {
        backend: BackendKind::LinuxKvm,
        runtime_version: format!(
            "kvm-api-{}-vcpu-mmap-{}",
            probe.api_version(),
            probe.vcpu_mmap_size()
        ),
        production_ready: false,
    })
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn probe_kvm() -> Result<BackendProbe, LocalFailure> {
    Err(LocalFailure::new(LocalFailureKind::UnsupportedTarget))
}
