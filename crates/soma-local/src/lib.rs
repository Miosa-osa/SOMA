#![doc = "Durable local state and target-gated host adapters for SOMA."]
#![forbid(unsafe_code)]

mod backend;
mod config;
mod error;
mod file_store;
mod runtime;

const MACHINE_HOST_DIRECTORY: &str = "machines";

#[cfg(test)]
mod test_support;

pub use backend::{BackendProbe, BackendSelection, MachineHosting, machine_hosting, probe_backend};

/// Waits for one launch capability, then serves that durable machine until it is released.
///
/// This is the body of the host process a managed Launch starts. It is public because the
/// command-line binary is the executable that re-enters here, and it is not a lifecycle
/// operation: a caller drives the machine it serves through [`LocalRuntime`], never through this.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[must_use]
pub fn host_machine(expected_socket: Option<&std::path::Path>) -> i32 {
    backend::host_machine(expected_socket)
}

/// Creates sterile machine-host processes before a hosted KVM service accepts traffic.
///
/// A sterile host owns no VM or Instance and only removes process creation from Launch latency.
///
/// # Errors
///
/// Returns a backend-unavailable failure when the bounded process set cannot be created.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub fn prewarm_machine_hosts(target: usize) -> Result<(), LocalFailure> {
    backend::prewarm_machine_hosts(target)
        .map_err(|_| LocalFailure::new(LocalFailureKind::BackendUnavailable))
}

/// Non-KVM targets have no machine-host process to prepare.
///
/// # Errors
///
/// This target-gated compatibility implementation cannot fail.
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub const fn prewarm_machine_hosts(_target: usize) -> Result<(), LocalFailure> {
    Ok(())
}

/// A durable machine host is a Linux `x86_64` capability, and no other target can serve one.
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
#[must_use]
pub const fn host_machine(_expected_socket: Option<&std::path::Path>) -> i32 {
    1
}
pub use config::LocalRuntimeConfig;
pub use error::{LocalFailure, LocalFailureKind};
pub use file_store::FileStateStore;
pub use runtime::LocalRuntime;
