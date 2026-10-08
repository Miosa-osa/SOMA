//! The two image-shaped live proofs: a tiny busybox machine and the node:22 machine.
//!
//! Split from the harness root so each file stays within the repository's source ceiling.

use std::path::Path;

use crate::{
    live::{BUSYBOX, boot_generation, serialize_live_proof},
    x86_64_sandbox_boot_generation as generation,
    x86_64_sandbox_boot_host::{assert_proof, require_kvm},
    x86_64_sandbox_boot_session as session,
};

const NODE: &str = "node:22";
/// The tree digest the node:22 export had when this proof was written.
const MAC_NODE_TREE_DIGEST: &str =
    "sha256:5dac6c571b970375a978c3f2f8777883e5bdd582fb4b43a5b872f929a2c7adf6";

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and Docker"]
fn tiny_generation_boots_authenticates_and_executes_one_command() {
    let _serialized = serialize_live_proof();
    require_kvm();
    let command = session::Command {
        program: b"/bin/busybox",
        arguments: &[b"uname", b"-a"],
        timeout_millis: 10_000,
        output_bytes: 65_536,
    };
    let proof = boot_generation(
        "busybox",
        BUSYBOX,
        "SOMA_OCI_BUSYBOX_LAYOUT",
        generation::Shape::new(256, 64, 1),
        &command,
    )
    .expect("prerequisite failed: the busybox OCI layout could not be exported; install Docker or set SOMA_OCI_BUSYBOX_LAYOUT");
    assert_proof(&proof);
    let stdout = String::from_utf8_lossy(&proof.executed.stdout);
    assert!(stdout.starts_with("Linux soma-"), "stdout={stdout:?}");
    assert!(stdout.contains("6.12.107-soma-v1"));
    assert!(stdout.contains("x86_64"));
    assert!(proof.executed.stderr.is_empty());
}

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and Docker with node:22"]
fn node_22_generation_boots_authenticates_and_reports_its_version() {
    let _serialized = serialize_live_proof();
    require_kvm();
    let command = session::Command {
        program: b"/usr/local/bin/node",
        arguments: &[b"--version"],
        timeout_millis: 30_000,
        output_bytes: 65_536,
    };
    let proof = boot_generation(
        "node22",
        NODE,
        "SOMA_OCI_NODE_LAYOUT",
        generation::Shape::new(1024, 1024, 1),
        &command,
    )
        .expect("prerequisite failed: the node:22 OCI layout could not be exported; set SOMA_OCI_NODE_LAYOUT");
    assert_proof(&proof);
    let stdout = String::from_utf8_lossy(&proof.executed.stdout);
    assert!(stdout.starts_with("v22."), "stdout={stdout:?}");
    let _ = Path::new(MAC_NODE_TREE_DIGEST);
}
