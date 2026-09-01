//! What `retryable` promises, and what refuses a launch this process cannot host.
//!
//! `retryable` is read by clients as permission to send the identical request again. A condition
//! that no amount of asking can clear must therefore report `false`, or a well behaved client
//! loops against it forever. These cases are the ones that used to say otherwise.

use soma::{
    BackendFailureKind, FailurePhase, ManagedFailure, RunFailureKind, StateStoreFailureKind,
};
use soma_local::{LocalFailureKind, MachineHosting};

use crate::exit::ProcessExit;

use super::{backend_failure_details, failure_details, local_failure, managed_failure, not_hosted};

const INSTANCE: &str = "22222222222222222222222222222222";

fn instance() -> soma::InstanceId {
    soma::InstanceId::new(INSTANCE).expect("a valid instance identity")
}

#[test]
fn an_unavailable_backend_capability_is_not_retryable() {
    let (body, exit) =
        backend_failure_details(FailurePhase::Launch, BackendFailureKind::Unavailable);

    assert_eq!(body.code, "backend_unavailable");
    assert!(
        !body.retryable,
        "a capability this host does not have does not appear because the caller asked again"
    );
    assert_eq!(exit, ProcessExit::CapabilityUnavailable);
}

#[test]
fn an_unavailable_local_backend_is_not_retryable() {
    let execution = local_failure("run", LocalFailureKind::BackendUnavailable);
    let error = execution
        .response
        .error()
        .expect("a refusal carries an error");

    assert_eq!(error.code, "backend_unavailable");
    assert!(!error.retryable);
    assert_eq!(execution.exit, ProcessExit::CapabilityUnavailable);
}

#[test]
fn only_a_clearing_state_store_condition_is_retryable() {
    let permanent = [
        StateStoreFailureKind::Corrupt,
        StateStoreFailureKind::InvalidRecord,
        StateStoreFailureKind::UnsupportedVersion,
        StateStoreFailureKind::CapacityExceeded,
    ];

    for kind in permanent {
        let (body, _) = failure_details(RunFailureKind::StateStore { kind });
        assert_eq!(body.code, "state_store_failure");
        assert!(!body.retryable, "{kind:?} cannot clear on its own");

        let execution = managed_failure(
            "machine.launch",
            instance(),
            &ManagedFailure::StateStore(kind),
        );
        let error = execution
            .response
            .error()
            .expect("a refusal carries an error");
        assert!(!error.retryable, "{kind:?} cannot clear on its own");
    }

    for kind in [
        StateStoreFailureKind::Conflict,
        StateStoreFailureKind::Unavailable,
    ] {
        let (body, _) = failure_details(RunFailureKind::StateStore { kind });
        assert!(body.retryable, "{kind:?} clears without operator action");
    }
}

#[test]
fn a_state_store_that_cannot_be_opened_reports_its_own_condition() {
    let corrupt = local_failure(
        "run",
        LocalFailureKind::StateStore(StateStoreFailureKind::Corrupt),
    );
    let contended = local_failure(
        "run",
        LocalFailureKind::StateStore(StateStoreFailureKind::Conflict),
    );

    assert!(!corrupt.response.error().expect("an error").retryable);
    assert!(contended.response.error().expect("an error").retryable);
}

#[test]
fn a_launch_this_process_cannot_host_is_refused_rather_than_reported_ready() {
    let execution = not_hosted("machine.launch");
    let error = execution
        .response
        .error()
        .expect("a refusal carries an error");

    assert_eq!(execution.response.status(), "error");
    assert!(execution.response.result().is_none());
    assert_eq!(error.code, "machine_not_hosted");
    assert!(!error.retryable);
    assert!(error.message.contains("soma run"));
    assert_eq!(execution.exit, ProcessExit::CapabilityUnavailable);
}

/// Every backend addresses its machine by Instance identity from a process that need not be the
/// one that launched it: KVM through a host process on a socket named by the Instance, Docker and
/// Apple `container` through a runtime service holding a machine named after the Instance. This
/// test is what fails when a backend arrives that does not.
#[test]
fn every_backend_hands_back_an_identity_a_later_process_can_use() {
    for backend in [
        soma::BackendKind::LinuxKvm,
        soma::BackendKind::MacosVirtualization,
        soma::BackendKind::DockerContainer,
        soma::BackendKind::Remote,
    ] {
        assert_eq!(
            soma_local::machine_hosting(backend),
            MachineHosting::OutlivesProcess,
            "{backend:?} hands back an identity no later process can address"
        );
    }
}
