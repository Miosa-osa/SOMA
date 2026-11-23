//! Open the configured backend and decide which network policies it can serve.

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use super::kvm;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use super::macos;
use super::{
    BackendSelection, LocalBackend, LocalFailure, LocalFailureKind, PathBuf, docker,
    resolve_selection,
};

impl LocalBackend {
    /// Whether this backend can give a machine the network `policy` describes, decided the way
    /// its own launch decides it.
    pub(crate) fn serves_network(&self, policy: &soma::NetworkPolicy) -> bool {
        match self {
            Self::Docker(_) => docker::network_mode(policy).is_some(),
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            Self::Macos(_) => macos::serves_network(policy),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            Self::Kvm(backend) => backend.serves_network(policy),
        }
    }

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
