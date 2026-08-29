#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
use soma_kvm::{KVM_API_VERSION, probe};

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
#[ignore = "requires a Linux x86_64 or ARM64 runner with accessible /dev/kvm"]
fn opens_dev_kvm_and_reports_required_capabilities() {
    let descriptors_before = open_descriptor_count();
    let report = probe().expect("the host must satisfy SOMA's required KVM capability contract");
    let descriptors_after = open_descriptor_count();

    assert_eq!(report.api_version(), KVM_API_VERSION);
    assert!(report.vcpu_mmap_size() > 0);
    assert_eq!(descriptors_after, descriptors_before);
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn open_descriptor_count() -> usize {
    std::fs::read_dir("/proc/self/fd")
        .expect("the KVM live-test host must mount procfs")
        .count()
}

#[cfg(not(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
)))]
#[test]
fn reports_an_unsupported_build_target_without_claiming_kvm_evidence() {
    assert!(!std::hint::black_box(soma_kvm::SUPPORTED_TARGET));
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
const _: () = assert!(soma_kvm::SUPPORTED_TARGET);

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn reports_linux_kvm_architectures_as_probe_capable_without_opening_kvm() {
    assert!(std::hint::black_box(soma_kvm::SUPPORTED_TARGET));
}
