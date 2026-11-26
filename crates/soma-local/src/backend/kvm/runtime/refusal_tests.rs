//! A host that has no room is told apart from a request no host would run.

use soma::BackendFailureKind;
use soma_hostd::{FailureCode, client::ClientError};

use crate::backend::kvm::runtime::launch_refusal;

/// A full host was answered `400 workload_rejected`, so the platform treated a sound request as
/// one to fix and never placed it on another host.
#[test]
fn a_host_with_no_room_is_a_capacity_refusal() {
    for code in [FailureCode::Capacity, FailureCode::Exhausted] {
        assert_eq!(
            launch_refusal(ClientError::Refused(code)),
            BackendFailureKind::CapacityExhausted
        );
    }
}

#[test]
fn a_conflict_is_still_a_conflict() {
    assert_eq!(
        launch_refusal(ClientError::Refused(FailureCode::Conflict)),
        BackendFailureKind::ResourceConflict
    );
}
