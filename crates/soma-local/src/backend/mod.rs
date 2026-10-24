mod hosting;
mod selection;

pub use hosting::{MachineHosting, machine_hosting};

mod clock;
mod docker;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod kvm;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod macos;

use std::path::PathBuf;

use soma::BackendKind;

use crate::{LocalFailure, LocalFailureKind};
use selection::resolve_selection;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) use kvm::host_machine;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) use kvm::open_tunnel;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) use kvm::{prewarm_machine_hosts, prewarm_status, set_prewarm_targets, start_prewarm};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) use kvm::{resize_machine, set_egress_machine};
pub(crate) use local::LocalBackend;

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

/// Where machines that outlive their launching process are addressed.
///
/// It sits under the same durable state root the lifecycle records are in, because an Instance
/// whose durable record says it is active and whose host is somewhere else would be two truths a
/// caller had to reconcile.
pub(crate) fn machine_host_directory(state_root: &std::path::Path) -> PathBuf {
    state_root.join(crate::file_store::MACHINE_HOST_DIRECTORY)
}

mod local;

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
