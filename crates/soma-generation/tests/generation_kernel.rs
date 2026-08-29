mod support;

use soma_generation::{
    CompileErrorKind, CompilePhase, Sha256Digest,
    kernel::{MINIMUM_LOAD_PADDR, verify_kernel},
    verify_kernel_config,
};
use support::generation::{kernel_config, synthetic_kernel};

const PH_NOTE_FILESZ: usize = 64 + 56 + 32;
const PH_NOTE_MEMSZ: usize = 64 + 56 + 40;

#[test]
fn kernel_with_pvh_note_inside_executable_segment_is_accepted() {
    let bytes = synthetic_kernel(Some(0x0100_0010), MINIMUM_LOAD_PADDR);
    let kernel = verify_kernel(&bytes).unwrap();
    assert_eq!(kernel.pvh_entry, 0x0100_0010);
    assert_eq!(kernel.segments.len(), 1);
    assert_eq!(kernel.segments[0].paddr, MINIMUM_LOAD_PADDR);
    assert!(kernel.segments[0].executable);
    assert_eq!(kernel.digest, Sha256Digest::of(&bytes));
    assert_eq!(kernel.size, bytes.len() as u64);
}

#[test]
fn kernel_without_pvh_note_is_rejected() {
    let error = verify_kernel(&synthetic_kernel(None, MINIMUM_LOAD_PADDR)).unwrap_err();
    assert_eq!(error.phase(), CompilePhase::VerifyKernel);
    assert_eq!(error.kind(), CompileErrorKind::Integrity);
}

#[test]
fn kernel_entry_outside_executable_segment_is_rejected() {
    let error =
        verify_kernel(&synthetic_kernel(Some(0x0200_0000), MINIMUM_LOAD_PADDR)).unwrap_err();
    assert_eq!(error.kind(), CompileErrorKind::Integrity);
}

#[test]
fn kernel_segment_below_minimum_load_address_is_rejected() {
    let error = verify_kernel(&synthetic_kernel(Some(0x0010_0010), 0x0010_0000)).unwrap_err();
    assert_eq!(error.kind(), CompileErrorKind::Integrity);
}

#[test]
fn kernel_with_two_pvh_notes_is_rejected() {
    let mut bytes = synthetic_kernel(Some(0x0100_0010), MINIMUM_LOAD_PADDR);
    let note = bytes[bytes.len() - 20..].to_vec();
    bytes.extend_from_slice(&note);
    bytes[PH_NOTE_FILESZ..PH_NOTE_FILESZ + 8].copy_from_slice(&40_u64.to_le_bytes());
    bytes[PH_NOTE_MEMSZ..PH_NOTE_MEMSZ + 8].copy_from_slice(&40_u64.to_le_bytes());
    assert_eq!(
        verify_kernel(&bytes).unwrap_err().kind(),
        CompileErrorKind::Integrity
    );
}

#[test]
fn non_executable_or_foreign_elf_is_unsupported() {
    let mut dynamic = synthetic_kernel(Some(0x0100_0010), MINIMUM_LOAD_PADDR);
    dynamic[16] = 3;
    assert_eq!(
        verify_kernel(&dynamic).unwrap_err().kind(),
        CompileErrorKind::Unsupported
    );
    let mut arm = synthetic_kernel(Some(0x0100_0010), MINIMUM_LOAD_PADDR);
    arm[18] = 0xb7;
    assert_eq!(
        verify_kernel(&arm).unwrap_err().kind(),
        CompileErrorKind::Unsupported
    );
    let mut class32 = synthetic_kernel(Some(0x0100_0010), MINIMUM_LOAD_PADDR);
    class32[4] = 1;
    assert_eq!(
        verify_kernel(&class32).unwrap_err().kind(),
        CompileErrorKind::Unsupported
    );
}

#[test]
fn truncated_or_garbage_elf_is_invalid() {
    let bytes = synthetic_kernel(Some(0x0100_0010), MINIMUM_LOAD_PADDR);
    assert_eq!(
        verify_kernel(&bytes[..100]).unwrap_err().kind(),
        CompileErrorKind::InvalidInput
    );
    assert_eq!(
        verify_kernel(b"not an elf").unwrap_err().kind(),
        CompileErrorKind::InvalidInput
    );
    let mut overflow = bytes.clone();
    overflow[32..40].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(verify_kernel(&overflow).is_err());
}

#[test]
fn kernel_config_requirements_are_enforced() {
    let good = kernel_config();
    let verified = verify_kernel_config(good.as_bytes()).unwrap();
    assert_eq!(verified.digest, Sha256Digest::of(good.as_bytes()));

    let missing = good.replace("CONFIG_EROFS_FS=y\n", "");
    assert_eq!(
        verify_kernel_config(missing.as_bytes()).unwrap_err().kind(),
        CompileErrorKind::Unsupported
    );
    let module = good.replace("CONFIG_EROFS_FS=y\n", "CONFIG_EROFS_FS=m\n");
    assert_eq!(
        verify_kernel_config(module.as_bytes()).unwrap_err().kind(),
        CompileErrorKind::Unsupported
    );
    let forbidden = format!("{good}CONFIG_PCI=y\n");
    assert_eq!(
        verify_kernel_config(forbidden.as_bytes())
            .unwrap_err()
            .kind(),
        CompileErrorKind::Unsupported
    );
    let relocated = good.replace("0x1000000", "0x200000");
    assert_eq!(
        verify_kernel_config(relocated.as_bytes())
            .unwrap_err()
            .kind(),
        CompileErrorKind::Unsupported
    );
    let malformed = format!("{good}garbage line\n");
    assert_eq!(
        verify_kernel_config(malformed.as_bytes())
            .unwrap_err()
            .kind(),
        CompileErrorKind::InvalidInput
    );
}
