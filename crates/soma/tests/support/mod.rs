mod backend;
mod gate;
mod network;
mod request;
mod terminal;

pub use backend::TestBackend;
pub use gate::CallGate;
pub use network::observed_network;
#[allow(
    unused_imports,
    reason = "each integration test selects a subset of shared request fixtures"
)]
pub use request::{run_request, run_request_with_output_limit};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "fixture modes are selected by separate integration tests"
)]
pub enum Mode {
    Happy,
    LaunchFailure,
    CommandFailure,
    /// The machine will not take the command at all, before running anything.
    WorkloadRejected,
    /// The guest's agent cannot decode the command, so it was refused with nothing sent.
    GuestAgentOutdated,
    /// The guest agent could not start the program, so no process ever ran.
    SpawnFailed,
    Timeout,
    /// The guest stopped the command because its output allowance ran out.
    OutputLimit,
    CommandIdentityMismatch,
    NonMonotonicCommand,
    CleanupFailure,
    /// The first cleanup fails part way; every later one succeeds.
    CleanupFailsOnce,
    FailureTimeRegression,
    CleanupFailureTimeRegression,
    BinaryOutput,
    CombinedOutputOverflow,
    Signaled,
    GracefulFallback,
    UnverifiedNetworkDenial,
    /// Every call succeeds, but the backend reports the machine's host gone, as after its worker
    /// was killed.
    Crashed,
}
