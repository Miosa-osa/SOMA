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
    Timeout,
    CommandIdentityMismatch,
    NonMonotonicCommand,
    CleanupFailure,
    FailureTimeRegression,
    CleanupFailureTimeRegression,
    BinaryOutput,
    CombinedOutputOverflow,
    Signaled,
    GracefulFallback,
    UnverifiedNetworkDenial,
}
