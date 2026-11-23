#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::{env, fs, io};

use soma::MachineName;
use soma_local::{
    BackendSelection, CheckpointArtifact, CheckpointFailure, CheckpointReceipt, LocalRuntime,
    LocalRuntimeConfig,
};

#[test]
#[ignore = "requires a captured sandbox and encrypted host fixture"]
fn exports_the_verified_artifacts_of_the_authorized_checkpoint() {
    let receipt: CheckpointReceipt = serde_json::from_slice(
        &fs::read(env::var_os("SOMA_EXPORT_TEST_RECEIPT").expect("fixture receipt")).unwrap(),
    )
    .unwrap();
    let owner = MachineName::parse(env::var("SOMA_EXPORT_TEST_OWNER").unwrap()).unwrap();
    let state_root = env::var_os("SOMA_PROOF_STATE_ROOT").expect("isolated state directory");
    let mut runtime = LocalRuntime::open(
        LocalRuntimeConfig::new(state_root)
            .with_backend(BackendSelection::Kvm)
            .with_hosted_machines(true),
    )
    .unwrap();
    let other = MachineName::parse("other-export-test-owner").unwrap();
    assert!(matches!(
        runtime.open_checkpoint_export(&other, &receipt),
        Err(CheckpointFailure::NotFound)
    ));
    let mut incorrect = receipt.clone();
    incorrect.state_digest = format!("sha256:{}", "00".repeat(32));
    assert!(matches!(
        runtime.open_checkpoint_export(&owner, &incorrect),
        Err(CheckpointFailure::CaptureRefused)
    ));

    let mut export = runtime.open_checkpoint_export(&owner, &receipt).unwrap();
    for (artifact, expected) in [
        (CheckpointArtifact::Memory, Some(receipt.memory_bytes)),
        (
            CheckpointArtifact::Overlay,
            receipt
                .overlay_digest
                .as_ref()
                .map(|_| receipt.overlay_bytes),
        ),
        (
            CheckpointArtifact::Workspace,
            receipt.volume.as_ref().map(|volume| volume.bytes),
        ),
    ] {
        assert_eq!(
            export.write_artifact(artifact, &mut io::sink()),
            Ok(expected)
        );
    }
    assert!(
        export
            .write_artifact(CheckpointArtifact::State, &mut io::sink())
            .unwrap()
            .is_some_and(|bytes| bytes > 0)
    );
}
