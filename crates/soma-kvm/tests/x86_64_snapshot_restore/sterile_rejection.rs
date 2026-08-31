//! What a prepared worker must refuse, proved against real KVM.

use soma_kvm::x86_64::{SterileRequest, restore_sterile};

use super::{fixture, require_kvm};

/// A machine that has restored everything except the two authorities an Instance owns.
fn sterile(fixture: &fixture::Fixture) -> soma_kvm::x86_64::Sterile {
    restore_sterile(SterileRequest {
        paths: fixture.paths.clone(),
        root: fixture.root(),
        overlay_capacity_bytes: fixture.overlay_capacity_bytes(),
        memory_bytes: fixture.ram_bytes,
        verify_artifacts: false,
    })
    .expect("a sterile machine restores without an Instance")
}

/// The guest has been told a capacity, so a head of another size cannot be substituted.
#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
fn a_head_of_the_wrong_shape_is_refused_and_the_worker_never_starts() {
    require_kvm();
    let fixture = fixture::shared();
    let descriptors_before = crate::x86_64_sandbox_boot_host::open_descriptor_count();
    let (_path, short) = fixture.private_head("sterile-short");
    short
        .set_len(fixture.overlay_capacity_bytes() / 2)
        .expect("shorten the head");

    let refused = sterile(&fixture).assign(short, 4);

    assert!(
        refused.is_err(),
        "a head of the wrong shape was accepted into a prepared worker"
    );
    // `assign` consumes the machine, so a refusal releases it rather than returning a
    // half-assigned worker to any caller.
    assert_eq!(
        crate::x86_64_sandbox_boot_host::open_descriptor_count(),
        descriptors_before,
        "a refused assignment leaked the worker's descriptors"
    );
}

/// Zero, one, and two are reserved, and the all-ones identifier is never assignable.
#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
fn an_unassignable_context_identifier_is_refused_and_the_worker_never_starts() {
    require_kvm();
    let fixture = fixture::shared();
    for reserved in [0_u32, 1, 2, u32::MAX] {
        let descriptors_before = crate::x86_64_sandbox_boot_host::open_descriptor_count();
        let (_path, head) = fixture.private_head("sterile-cid");

        let refused = sterile(&fixture).assign(head, reserved);

        assert!(
            refused.is_err(),
            "context identifier {reserved} was accepted into a prepared worker"
        );
        assert_eq!(
            crate::x86_64_sandbox_boot_host::open_descriptor_count(),
            descriptors_before,
            "a refused assignment leaked the worker's descriptors"
        );
    }
}
