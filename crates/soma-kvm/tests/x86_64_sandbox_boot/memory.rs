//! Live proof that a sixteen-gigabyte machine's memory is all there and writable.
//!
//! The guest's `/proc/meminfo` total says what the kernel was told, not that both KVM slots back
//! the addresses: a machine with the second slot missing still reports the high range, and it
//! faults only when something writes there. This boots the eight-vCPU shape, writes most of the
//! RAM through a tmpfs, reads every byte of it back, and reports the sizes and the digest, so a
//! slot that is not backed fails the write, the read back, or the accounting.

use crate::{
    live::{BUSYBOX, boot_generation, serialize_live_proof},
    x86_64_sandbox_boot_generation as generation,
    x86_64_sandbox_boot_host::{assert_proof, require_kvm},
    x86_64_sandbox_boot_memory_fill as fill, x86_64_sandbox_boot_session as session,
};

/// The machine contract v2 memory gate: sixteen gigabytes, every byte of them writable.
#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and Docker"]
fn an_eight_vcpu_generation_writes_across_all_sixteen_gigabytes() {
    let _serialized = serialize_live_proof();
    require_kvm();
    let command = session::Command {
        program: b"/bin/sh",
        arguments: &[b"-c", fill::SCRIPT.as_bytes()],
        // Filling and reading back fourteen gigabytes is memory bandwidth work, so the step gets
        // the machine's own patience rather than the ten seconds a small command needs: too short
        // a bound here would read as a failed write.
        timeout_millis: 300_000,
        output_bytes: 65_536,
    };
    let proof = boot_generation(
        "busybox-fill",
        BUSYBOX,
        "SOMA_OCI_BUSYBOX_LAYOUT",
        generation::Shape::new(fill::MEMORY_MIB, 1024, 8),
        &command,
    )
    .expect("prerequisite failed: the busybox OCI layout could not be exported; set SOMA_OCI_BUSYBOX_LAYOUT");
    assert_proof(&proof);
    let stdout = String::from_utf8_lossy(&proof.executed.stdout);
    fill::assert_wrote_all_the_ram(&stdout);
}
