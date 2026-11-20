//! The prune request's own rules, which hold on every target.

use soma::{
    BackendKind, GenerationId, InstanceId, MachineListing, SandboxEntry, SandboxLiveness,
    SandboxPhase, StateStoreFailureKind, UnreadableRecord,
};

use super::{InstanceHolds, PruneError, PruneRequest};

fn request(keep: &[&str], apply: bool) -> PruneRequest {
    PruneRequest {
        keep: keep
            .iter()
            .map(|reference| (*reference).to_owned())
            .collect(),
        apply,
        keep_previous: true,
    }
}

fn instance(fill: char) -> InstanceId {
    InstanceId::new(fill.to_string().repeat(32)).expect("instance")
}

fn generation(fill: char) -> GenerationId {
    GenerationId::new(format!("sha256:{}", fill.to_string().repeat(64))).expect("generation")
}

fn entry(fill: char, backend: BackendKind, generation_id: Option<GenerationId>) -> SandboxEntry {
    SandboxEntry::new(
        instance(fill),
        SandboxPhase::Active,
        backend,
        None,
        SandboxLiveness::Unknown,
    )
    .with_generation(generation_id)
}

#[test]
fn an_applied_prune_with_nothing_kept_is_refused() {
    assert_eq!(request(&[], true).validate(), Err(PruneError::KeepRequired));
}

#[test]
fn a_dry_run_with_nothing_kept_is_allowed() {
    assert_eq!(request(&[], false).validate(), Ok(()));
}

#[test]
fn a_kept_value_that_is_not_a_reference_is_refused() {
    assert!(matches!(
        request(&["has space:1"], false).validate(),
        Err(PruneError::InvalidKeep(_))
    ));
}

#[test]
fn a_prewarm_floor_key_keeps_its_reference_and_a_digest_is_kept_whole() {
    let kept = request(
        &[
            "soma-default:G20-sr1@2048",
            "node:22",
            "node@sha256:0123456789abcdef",
        ],
        true,
    )
    .kept();
    let kept: Vec<&str> = kept.iter().map(String::as_str).collect();
    assert_eq!(
        kept,
        [
            "node:22",
            "node@sha256:0123456789abcdef",
            "soma-default:G20-sr1"
        ]
    );
}

#[test]
fn holds_name_each_kvm_generation_and_count_everything_unknown_as_unattributed() {
    let listing = MachineListing::with_unreadable(
        vec![
            entry('a', BackendKind::LinuxKvm, Some(generation('1'))),
            entry('b', BackendKind::LinuxKvm, None),
            entry('c', BackendKind::DockerContainer, None),
        ],
        vec![UnreadableRecord::new(
            instance('d'),
            StateStoreFailureKind::Corrupt,
        )],
    );
    let holds = InstanceHolds::from_listing(&listing);
    assert_eq!(holds.held, vec![(instance('a'), generation('1'))]);
    // A KVM record with no Generation and an undecodable record could each hold anything; a
    // record another backend wrote holds nothing in the prepared store.
    assert_eq!(holds.unattributed, vec![instance('b'), instance('d')]);
}

#[test]
fn a_request_keeps_the_rollback_target_unless_it_says_otherwise() {
    assert!(PruneRequest::default().keep_previous);
    let empty: PruneRequest = serde_json::from_str("{}").expect("request");
    assert!(empty.keep_previous);
    let dropping: PruneRequest =
        serde_json::from_str(r#"{"keep":["soma-default:G10-sr1"],"keep_previous":false}"#)
            .expect("request");
    assert!(!dropping.keep_previous);
}
