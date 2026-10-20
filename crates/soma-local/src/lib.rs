#![doc = "Durable local state and target-gated host adapters for SOMA."]
#![forbid(unsafe_code)]

mod backend;
mod config;
mod error;
mod file_store;
mod prewarm;
mod egress;
mod resize;
mod runtime;
mod tunnel;

#[cfg(test)]
mod test_support;

pub use backend::{BackendProbe, BackendSelection, MachineHosting, machine_hosting, probe_backend};
pub use prewarm::{
    MAX_TARGET_COUNT, MAX_TARGETS, PrewarmError, PrewarmStatus, PrewarmTarget, Source,
    TargetStatus, UncoveredDemand,
};
pub use egress::{EgressSetFailure, EgressSetOutcome, EgressSetRequest};
pub use resize::{
    CpuResize, DimensionState, MemoryResize, ResizeFailure, ResizeOutcome, ResizeRequest,
};
pub use tunnel::TunnelFailure;

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

/// Prepares the pool of machines before a hosted KVM service accepts traffic.
///
/// A persisted prewarm set under `state_root` wins over the environment's single shape; without
/// one the environment applies as it always did, and `workers` is the number of spares.
///
/// # Errors
///
/// Returns a backend-unavailable failure when the bounded process set cannot be created.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub fn start_prewarm(state_root: &std::path::Path, workers: usize) -> Result<(), LocalFailure> {
    backend::start_prewarm(state_root, workers)
        .map_err(|_| LocalFailure::new(LocalFailureKind::BackendUnavailable))
}

/// Non-KVM targets prepare nothing.
///
/// # Errors
///
/// This target-gated compatibility implementation cannot fail.
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub const fn start_prewarm(
    _state_root: &std::path::Path,
    _workers: usize,
) -> Result<(), LocalFailure> {
    Ok(())
}

/// Replaces the desired prewarm set, whole or not at all, and reads it back.
///
/// # Errors
///
/// Returns the named refusal: an unknown reference, an invalid or over-budget set, or a set that
/// could not be persisted.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub fn set_prewarm_targets(targets: &[PrewarmTarget]) -> Result<PrewarmStatus, PrewarmError> {
    backend::set_prewarm_targets(targets)
}

/// Non-KVM targets prepare nothing, so a set is refused.
///
/// # Errors
///
/// Always [`PrewarmError::Unsupported`].
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub const fn set_prewarm_targets(
    _targets: &[PrewarmTarget],
) -> Result<PrewarmStatus, PrewarmError> {
    Err(PrewarmError::Unsupported)
}

/// The desired prewarm set and what exists.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[must_use]
pub fn prewarm_status() -> PrewarmStatus {
    backend::prewarm_status()
}

/// Non-KVM targets have nothing prepared.
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
#[must_use]
pub const fn prewarm_status() -> PrewarmStatus {
    PrewarmStatus {
        source: Source::EnvDefault,
        persisted_error: None,
        budget_mib: 0,
        committed_mib: 0,
        unprimed_target: 0,
        unprimed_prepared: 0,
        targets: Vec::new(),
        uncovered_demand: Vec::new(),
    }
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
