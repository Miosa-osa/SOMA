use std::{
    env,
    path::PathBuf,
    time::{Duration, Instant},
};

use super::*;

const UNREACHABLE_SENTINEL: &[u8] = b"SOMA_SENTINEL_NEVER_EMITTED";

#[test]
fn boot_evidence_debug_redacts_guest_console() {
    let evidence = Arm64BootEvidence {
        console: b"guest secret".to_vec(),
    };

    assert_eq!(
        format!("{evidence:?}"),
        "Arm64BootEvidence { console_len: 12 }"
    );
}

#[test]
#[ignore = "requires Linux ARM64, /dev/kvm, and explicit kernel/initramfs fixtures"]
fn boots_linux_arm64_pid1_and_releases_descriptors() {
    let kernel = fixture("SOMA_KVM_ARM64_KERNEL");
    let initramfs = fixture("SOMA_KVM_ARM64_INITRAMFS");
    let descriptors_before = open_descriptor_count();
    let started = Instant::now();

    let evidence = boot_arm64_fixture(&kernel, &initramfs)
        .expect("the nested ARM64 KVM guest must reach the fixture PID1 sentinel");
    let elapsed = started.elapsed();
    assert!(
        evidence
            .console()
            .windows(ARM64_BOOT_SENTINEL.len())
            .any(|window| window == ARM64_BOOT_SENTINEL.as_bytes())
    );
    let descriptors_after = open_descriptor_count();
    eprintln!(
        "ARM64 cold boot: elapsed_ms={}, fd_before={descriptors_before}, fd_after={descriptors_after}",
        elapsed.as_millis()
    );
    assert_eq!(descriptors_after, descriptors_before);
}

#[test]
#[ignore = "requires Linux ARM64, /dev/kvm, and explicit kernel/initramfs fixtures"]
fn watchdog_stops_a_guest_that_never_emits_the_expected_sentinel() {
    let kernel = fixture("SOMA_KVM_ARM64_KERNEL");
    let initramfs = fixture("SOMA_KVM_ARM64_INITRAMFS");
    let descriptors_before = open_descriptor_count();
    let started = Instant::now();

    let error = boot_with(
        &kernel,
        &initramfs,
        UNREACHABLE_SENTINEL,
        Duration::from_secs(5),
    )
    .unwrap_err();
    let elapsed = started.elapsed();

    assert!(error.to_string().contains("timed out"));
    let descriptors_after = open_descriptor_count();
    eprintln!(
        "ARM64 forced timeout: elapsed_ms={}, fd_before={descriptors_before}, fd_after={descriptors_after}",
        elapsed.as_millis()
    );
    assert_eq!(descriptors_after, descriptors_before);
}

fn fixture(variable: &str) -> PathBuf {
    let path = PathBuf::from(
        env::var_os(variable)
            .unwrap_or_else(|| panic!("{variable} must name an explicit fixture path")),
    );
    assert!(path.is_absolute(), "{variable} must be absolute");
    assert!(path.is_file(), "{variable} must name an existing file");
    path
}

fn open_descriptor_count() -> usize {
    std::fs::read_dir("/proc/self/fd")
        .expect("the KVM live-test host must mount procfs")
        .count()
}
