mod clock;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod kvm;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod macos;

use std::path::PathBuf;

use soma::{
    Backend, BackendFailure, BackendKind, CleanupObservation, CleanupRequest, CommandObservation,
    ExecutionRequest, InspectionObservation, InspectionRequest, LaunchObservation, LaunchRequest,
    ResolutionObservation, ResolutionRequest,
};

use crate::{LocalFailure, LocalFailureKind};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BackendSelection {
    #[default]
    Auto,
    Macos,
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
        BackendSelection::Kvm => probe_kvm(),
        BackendSelection::Auto => unreachable!("auto selection is resolved before probing"),
    }
}

pub(crate) enum LocalBackend {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    Macos(macos::MacBackend),
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    Kvm(kvm::KvmBackend),
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
type PreparedWorkload = macos::MacPreparedWorkload;

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
pub(crate) enum PreparedWorkload {}

impl LocalBackend {
    pub(crate) fn open(
        selection: BackendSelection,
        explicit_runtime: Option<PathBuf>,
    ) -> Result<(Self, BackendSelection), LocalFailure> {
        let resolved = resolve_selection(selection)?;
        match resolved {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            BackendSelection::Macos => macos::MacBackend::open(explicit_runtime)
                .map(|backend| (Self::Macos(backend), resolved))
                .map_err(|_| LocalFailure::new(LocalFailureKind::BackendUnavailable)),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            BackendSelection::Kvm => Ok((Self::Kvm(kvm::KvmBackend::new()), resolved)),
            _ => Err(LocalFailure::new(LocalFailureKind::UnsupportedTarget)),
        }
    }
}

impl Backend for LocalBackend {
    type PreparedWorkload = PreparedWorkload;

    fn kind(&self) -> BackendKind {
        match self {
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
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            Self::Macos(backend) => backend.resolve(request),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            Self::Kvm(backend) => Err(backend.unavailable(request.operation_id())),
        }
    }

    fn launch(
        &mut self,
        request: LaunchRequest<'_, Self::PreparedWorkload>,
    ) -> Result<LaunchObservation, BackendFailure> {
        match self {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            Self::Macos(backend) => backend.launch(&request),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            Self::Kvm(backend) => Err(backend.unavailable(request.operation_id())),
        }
    }

    fn execute(
        &mut self,
        request: ExecutionRequest<'_>,
    ) -> Result<CommandObservation, BackendFailure> {
        match self {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            Self::Macos(backend) => backend.execute(request),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            Self::Kvm(backend) => Err(backend.unavailable(request.operation_id())),
        }
    }

    fn inspect(
        &mut self,
        request: InspectionRequest<'_>,
    ) -> Result<InspectionObservation, BackendFailure> {
        match self {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            Self::Macos(backend) => backend.inspect(request),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            Self::Kvm(backend) => Err(backend.unavailable(request.operation_id())),
        }
    }

    fn cleanup(
        &mut self,
        request: CleanupRequest<'_>,
    ) -> Result<CleanupObservation, BackendFailure> {
        match self {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            Self::Macos(backend) => backend.cleanup(request),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            Self::Kvm(backend) => Err(backend.unavailable(request.operation_id())),
        }
    }
}

fn resolve_selection(selection: BackendSelection) -> Result<BackendSelection, LocalFailure> {
    if selection != BackendSelection::Auto {
        return Ok(selection);
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
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
