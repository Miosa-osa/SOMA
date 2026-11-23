#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use soma::{InstanceId, MachineRelease};
use soma_local::{BackendSelection, LocalRuntime, LocalRuntimeConfig};

#[test]
#[ignore = "requires a stopped hosted KVM Instance with a published checkpoint in an isolated fixture"]
fn a_published_checkpoint_keeps_the_stopped_record() {
    let root = std::env::var_os("SOMA_RETENTION_PROOF_STATE").expect("isolated state root");
    let instance = InstanceId::new(
        std::env::var("SOMA_RETENTION_PROOF_INSTANCE").expect("stopped fixture Instance"),
    )
    .expect("valid Instance");
    let mut runtime = LocalRuntime::open(
        LocalRuntimeConfig::new(root)
            .with_backend(BackendSelection::Kvm)
            .with_hosted_machines(true),
    )
    .expect("KVM runtime for isolated fixture");
    assert!(
        runtime
            .terminal_census()
            .unwrap()
            .terminal()
            .contains(&instance),
        "the sweep would have considered the stopped fixture"
    );
    assert_eq!(
        runtime.release_machine(&instance).unwrap(),
        MachineRelease::Kept
    );
    assert!(
        runtime
            .terminal_census()
            .unwrap()
            .terminal()
            .contains(&instance)
    );

    assert!(
        runtime
            .list_machines()
            .unwrap()
            .iter()
            .all(|entry| entry.instance_id() != &instance),
        "the stopped record must be retained even though the active listing does not show it"
    );
    println!("Published checkpoint kept the stopped record.");
}

#[test]
#[ignore = "requires an isolated synthetic prepared inventory and a published checkpoint fixture"]
fn a_published_checkpoint_protects_its_prepared_generation() {
    let instance =
        std::env::var("SOMA_RETENTION_PROOF_INSTANCE").expect("stopped fixture Instance");
    let generation = std::env::var("SOMA_RETENTION_PROOF_GENERATION").expect("fixture Generation");
    let request = soma_local::PruneRequest {
        keep: vec!["node:22-retention-pin".to_owned()],
        apply: false,
        keep_previous: false,
    };
    let plan =
        soma_local::prune_prepared(&request, &mut || Ok(soma_local::InstanceHolds::default()))
            .expect("prepared Generation prune dry run");
    assert!(
        plan.refused.iter().any(|entry| {
            entry.reference.as_deref() == Some("node:22")
                && entry.generation_id.as_deref() == Some(generation.as_str())
                && entry.reason == soma_local::PruneReason::HasInstances
                && entry.detail.iter().any(|holder| holder == &instance)
        }),
        "the checkpoint must be reported as a Generation holder: {plan:?}"
    );
    println!("Published checkpoint protected its prepared Generation from pruning.");
}
