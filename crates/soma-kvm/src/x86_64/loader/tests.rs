use super::*;
use crate::cmdline::{self, BootNonce};
use crate::virtio::GuestMemory as _;
use crate::x86_64::{
    elf::synthetic::{PF_R, Segment, SyntheticElf},
    error::MachineErrorKind,
    layout::{GuestLayout, MIN_RAM_BYTES},
};

fn ram() -> GuestRam {
    GuestRam::map(GuestLayout::new(MIN_RAM_BYTES).unwrap()).unwrap()
}

fn line(initramfs: bool, nonce: Option<&BootNonce>) -> String {
    cmdline::compose(initramfs, nonce)
}

#[test]
fn loads_kernel_and_places_initramfs_top_down() {
    let entry = u32::try_from(KERNEL_START).unwrap() + 8;
    let image = SyntheticElf::kernel(entry).build();
    let initramfs = vec![0xaa_u8; 5000];
    let nonce = BootNonce::new([1; 8]);
    let loaded = load_kernel(
        &mut ram(),
        &image,
        Some(&initramfs),
        &line(true, Some(&nonce)),
        crate::contract::MachineContract::V1,
        1,
    )
    .unwrap();
    assert_eq!(loaded.entry, u64::from(entry));
    assert_eq!(loaded.kernel_end, KERNEL_START + 64 + 4096);
    let expected_start = (MIN_RAM_BYTES - 5000) & !(PAGE_SIZE - 1);
    assert_eq!(loaded.initramfs, Some((expected_start, 5000)));
    assert!(
        loaded
            .cmdline
            .ends_with("rdinit=/init soma.nonce=0101010101010101")
    );
}

#[test]
fn a_contract_that_publishes_an_mp_table_writes_one_for_every_processor() {
    let entry = u32::try_from(KERNEL_START).unwrap();
    let image = SyntheticElf::kernel(entry).build();
    let mut version_two = ram();
    load_kernel(
        &mut version_two,
        &image,
        None,
        &line(false, None),
        crate::contract::MachineContract::V2,
        4,
    )
    .unwrap();
    let expected = crate::mptable::encode(4).unwrap();
    let mut written = vec![0_u8; expected.len()];
    version_two
        .shared()
        .read_bytes(
            crate::virtio::GuestAddress(crate::mptable::MP_TABLE_ADDRESS),
            &mut written,
        )
        .unwrap();
    assert_eq!(written, expected);
    // Version 1 declares no table, so the region stays zero.
    let mut version_one = ram();
    load_kernel(
        &mut version_one,
        &image,
        None,
        &line(false, None),
        crate::contract::MachineContract::V1,
        1,
    )
    .unwrap();
    let mut unwritten = vec![0_u8; expected.len()];
    version_one
        .shared()
        .read_bytes(
            crate::virtio::GuestAddress(crate::mptable::MP_TABLE_ADDRESS),
            &mut unwritten,
        )
        .unwrap();
    assert!(unwritten.iter().all(|byte| *byte == 0));
}

#[test]
fn loads_without_initramfs_or_nonce() {
    let entry = u32::try_from(KERNEL_START).unwrap();
    let image = SyntheticElf::kernel(entry).build();
    let loaded = load_kernel(
        &mut ram(),
        &image,
        None,
        &line(false, None),
        crate::contract::MachineContract::V1,
        1,
    )
    .unwrap();
    assert_eq!(loaded.initramfs, None);
    assert_eq!(loaded.cmdline, boot_info::DIAGNOSTIC_CMDLINE);
}

#[test]
fn rejects_segments_outside_ram_and_oversized_initramfs() {
    let entry = u32::try_from(KERNEL_START).unwrap();
    let mut elf = SyntheticElf::kernel(entry);
    elf.segments.push(Segment {
        address: MIN_RAM_BYTES - 8,
        data: vec![0; 16],
        extra_memory: 0,
        flags: PF_R,
    });
    let error = load_kernel(
        &mut ram(),
        &elf.build(),
        None,
        &line(false, None),
        crate::contract::MachineContract::V1,
        1,
    )
    .unwrap_err();
    assert_eq!(error.phase(), Phase::LoadGuest);
    assert!(matches!(error.kind(), MachineErrorKind::Invalid(_)));

    let image = SyntheticElf::kernel(entry).build();
    assert!(
        load_kernel(
            &mut ram(),
            &image,
            Some(&[]),
            &line(true, None),
            crate::contract::MachineContract::V1,
            1
        )
        .is_err()
    );
    let huge = vec![0_u8; usize::try_from(MIN_RAM_BYTES).unwrap()];
    assert!(
        load_kernel(
            &mut ram(),
            &image,
            Some(&huge[..huge.len() - 4096]),
            &line(true, None),
            crate::contract::MachineContract::V1,
            1
        )
        .is_err()
    );
}

#[test]
fn rejects_an_initramfs_that_would_cover_the_kernel() {
    let entry = u32::try_from(KERNEL_START).unwrap();
    let mut elf = SyntheticElf::kernel(entry);
    elf.segments[0].extra_memory = MIN_RAM_BYTES - KERNEL_START - 64 - 8192;
    let initramfs = vec![1_u8; 12288];
    let error = load_kernel(
        &mut ram(),
        &elf.build(),
        Some(&initramfs),
        &line(true, None),
        crate::contract::MachineContract::V1,
        1,
    )
    .unwrap_err();
    assert!(error.to_string().contains("does not fit"));
}

#[test]
fn elf_rejections_surface_as_typed_load_errors() {
    let error = load_kernel(
        &mut ram(),
        b"not an elf",
        None,
        &line(false, None),
        crate::contract::MachineContract::V1,
        1,
    )
    .unwrap_err();
    assert_eq!(error.phase(), Phase::LoadGuest);
    assert!(matches!(error.kind(), MachineErrorKind::Elf(_)));
}
