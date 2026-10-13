//! The pre-volume identity pins.
//!
//! The durable volume added a seventh slot to the device vocabulary, a seventh device kind, and a
//! seventh expected entry to the compatibility profile. None of that may reach a Generation that
//! declares no volume: its device contract, its kernel command line, and its canonical manifest
//! must stay byte for byte what they were certified as.
//!
//! Every constant below was captured by running this code at the trunk commit `cf471022ec`, the
//! last commit before the volume slot existed, and printing the value.

use std::fmt::Write as _;

use sha2::{Digest as _, Sha256};
use soma_kvm::DeviceSet;

use super::{encode_manifest, fixture};

/// The SHA-256 of the encoded version 3 fixture manifest, which declares no durable volume.
const PRE_VOLUME_MANIFEST_SHA256: &str =
    "137da82583c2dce91d32a2a3e5361f7dd7bf7bbcfd6966c3f39de00e5c00d6f5";
/// Its length, which the digest alone would not localize to a field.
const PRE_VOLUME_MANIFEST_LEN: usize = 1660;
/// The device set every fixture below policy 4 builds: an overlay, no network, no memory device.
const PRE_VOLUME_DEVICES: DeviceSet = DeviceSet::new(true, false);
/// The SHA-256 of the canonical device-contract statement of [`PRE_VOLUME_DEVICES`].
const PRE_VOLUME_CONTRACT_SHA256: &str =
    "f88c4f926c6cca8948da4c8211ce8ebb587f869136418c4b3df2cde49a14081e";
/// Its length, for the same reason.
const PRE_VOLUME_CONTRACT_LEN: usize = 635;
/// The kernel command line the version 3 fixture was certified with, which declares the memory
/// device and no volume.
const PRE_VOLUME_COMMAND_LINE: &str = "console=ttyS0 reboot=k panic=1 nomodule \
random.trust_cpu=off pci=off acpi=off cryptomgr.notests virtio_mmio.device=4K@0xd0000000:5:0 \
virtio_mmio.device=4K@0xd0001000:6:1 virtio_mmio.device=4K@0xd0003000:8:3 \
virtio_mmio.device=4K@0xd0004000:9:4 virtio_mmio.device=4K@0xd0005000:10:5 rdinit=/init \
soma.lower=/dev/vda soma.upper=/dev/vdb";

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

#[test]
fn a_generation_that_declares_no_volume_is_byte_for_byte_the_one_it_was() {
    let manifest = fixture::profile_v3();
    assert!(!manifest.device_set().volume());
    let bytes = encode_manifest(&manifest).expect("encodes");
    assert_eq!(bytes.len(), PRE_VOLUME_MANIFEST_LEN);
    assert_eq!(
        sha256(&bytes),
        PRE_VOLUME_MANIFEST_SHA256,
        "the canonical manifest of a Generation with no volume must not move"
    );
}

#[test]
fn the_device_contract_of_a_machine_without_a_volume_is_unchanged() {
    let statement = crate::generation::contracts::device_contract_statement(PRE_VOLUME_DEVICES);
    assert_eq!(statement.len(), PRE_VOLUME_CONTRACT_LEN);
    assert_eq!(sha256(&statement), PRE_VOLUME_CONTRACT_SHA256);
    // The statement says which slots the machine has, so a slot line appearing here would change
    // the digest a snapshot's compatibility is judged by.
    let text = String::from_utf8(statement).expect("utf8");
    assert!(text.contains("slot0 mmio=0xd0000000"));
    assert!(
        !text.contains("slot5"),
        "the memory device was not declared"
    );
    assert!(!text.contains("slot6"), "no volume slot may appear");
    assert!(!text.contains("durable-volume"));
}

#[test]
fn the_kernel_command_line_of_a_machine_without_a_volume_is_unchanged() {
    let with_memory = PRE_VOLUME_DEVICES.with_mem(true);
    let certified = crate::generation::contracts::kernel_command_line_for(with_memory, 3)
        .expect("machine contract 3 has a line");
    assert_eq!(
        String::from_utf8(certified).expect("utf8"),
        PRE_VOLUME_COMMAND_LINE,
        "the certified version 3 line is unchanged, character for character"
    );

    // The volume would take the seventh slot, so its page never appears for a machine that
    // declared none. That is the "first unmapped page" property: it is a host-side table
    // property, and it must not reach the guest's view through any line.
    for devices in [
        PRE_VOLUME_DEVICES,
        PRE_VOLUME_DEVICES.with_mem(true),
        DeviceSet::new(true, true),
    ] {
        for version in [1, 2, 3] {
            let line = crate::generation::contracts::kernel_command_line_for(devices, version)
                .expect("every contract this build makes has a line");
            let text = String::from_utf8(line).expect("utf8");
            assert!(
                !text.contains("0xd0006000"),
                "version {version} names the volume page"
            );
            assert!(
                !text.contains("soma.volume="),
                "version {version} names the volume"
            );
        }
    }
}
