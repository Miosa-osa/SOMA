//! What a create's launch failure was, written where it happens.
//!
//! The answer and the journal carry only the failure's stable code, which is right for a client
//! but leaves an operator with `backend_failure` and nothing else: no phase, no kind, no duration.
//! That made a chronic, low-rate launch failure unattributable after the fact. The launch now
//! says, at the moment it fails, which phase and kind it was and how long it had run.

use std::time::Instant;

use soma::ManagedFailure;

use super::{SandboxId, failure_code};

/// Logs a failed launch with its phase, kind and duration, and returns the stable code the
/// answer carries.
pub(super) fn launch_failed(
    id: &SandboxId,
    started: Instant,
    failure: &ManagedFailure,
) -> &'static str {
    let code = failure_code(failure);
    eprintln!(
        "soma-api: runner create {id} launch failed after {} ms: {code} ({})",
        started.elapsed().as_millis(),
        detail(failure),
    );
    code
}

/// The failure beyond its code: the failing phase and backend kind when it carries them. It names
/// no guest data, only the engine's own classification.
fn detail(failure: &ManagedFailure) -> String {
    match failure {
        ManagedFailure::Operation(run) => format!("{:?}", run.kind()),
        ManagedFailure::Backend(kind) => format!("Backend {{ kind: {kind:?} }}"),
        ManagedFailure::State(state) => format!("State({state:?})"),
        ManagedFailure::StateStore(kind) => format!("StateStore({kind:?})"),
        ManagedFailure::ReplayUnavailable(_) => "ReplayUnavailable".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backend_failure_names_its_kind() {
        let failure = ManagedFailure::Backend(soma::BackendFailureKind::Timeout);
        assert_eq!(detail(&failure), "Backend { kind: Timeout }");
    }
}
