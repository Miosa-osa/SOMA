#![doc = "Durable local state and target-gated host adapters for SOMA."]
#![forbid(unsafe_code)]

mod backend;
mod config;
mod error;
mod file_store;
mod runtime;

#[cfg(test)]
mod test_support;

pub use backend::{BackendProbe, BackendSelection, probe_backend};

/// Serves one durable machine at `socket` until it is released, returning the process status.
///
/// This is the body of the host process a managed Launch starts. It is public because the
/// command-line binary is the executable that re-enters here, and it is not a lifecycle
/// operation: a caller drives the machine it serves through [`LocalRuntime`], never through this.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[must_use]
pub fn host_machine(socket: &std::path::Path) -> i32 {
    backend::host_machine(socket)
}

/// A durable machine host is a Linux `x86_64` capability, and no other target can serve one.
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
#[must_use]
pub fn host_machine(_socket: &std::path::Path) -> i32 {
    1
}
pub use config::LocalRuntimeConfig;
pub use error::{LocalFailure, LocalFailureKind};
pub use file_store::FileStateStore;
pub use runtime::LocalRuntime;
