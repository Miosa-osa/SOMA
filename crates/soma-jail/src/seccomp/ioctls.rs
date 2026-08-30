//! The `ioctl` request allowlist, filtered on the request argument.
//!
//! Numbers are computed from the `_IOC` encoding with `KVMIO = 0xAE`; the Linux `x86_64` test
//! checks each structure size against `kvm-bindings`.
//! `TUNSETIFF` is deliberately absent: the broker configures the TAP before transfer.

use super::{Phase, policy::Provenance};
use crate::seccomp::policy::{Need, Source};

const KVMIO: u32 = 0xAE;
const WRITE: u32 = 1 << 30;
const READ: u32 = 2 << 30;

const fn io(nr: u32) -> u32 {
    (KVMIO << 8) | nr
}

const fn iow(nr: u32, size: u32) -> u32 {
    WRITE | (size << 16) | io(nr)
}

const fn ior(nr: u32, size: u32) -> u32 {
    READ | (size << 16) | io(nr)
}

const fn iowr(nr: u32, size: u32) -> u32 {
    READ | WRITE | (size << 16) | io(nr)
}

/// Which phases admit a request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IoctlPhases {
    StartupOnly,
    Both,
}

/// One admitted request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IoctlRule {
    pub name: &'static str,
    pub request: u32,
    pub phases: IoctlPhases,
    pub provenance: Provenance,
}

const fn rule(
    name: &'static str,
    request: u32,
    phases: IoctlPhases,
    provenance: Provenance,
) -> IoctlRule {
    IoctlRule {
        name,
        request,
        phases,
        provenance,
    }
}

const KVM: Provenance = Provenance::Measured(Source::SomaKvm);
const RUST: Provenance = Provenance::Measured(Source::RustRuntime);
const SNAPSHOT: Provenance = Provenance::Reserved(Need::SnapshotRestore);
const VIRTIO: Provenance = Provenance::Reserved(Need::VirtioDevices);

use IoctlPhases::{Both, StartupOnly};

/// The complete request table.
///
/// Measured entries come from `soma-kvm/src/x86_64` and `machine.rs`.
/// `KVM_IRQFD` is measured at startup and stays in steady state because device teardown
/// deassigns it, and `KVM_IOEVENTFD` is reserved for the virtio queue notification path.
/// `FIONBIO` is the only non-KVM request: the Rust runtime uses it for `set_nonblocking`.
/// `KVM_SET_USER_MEMORY_REGION` registers guest memory at startup and stays in steady state
/// because launch-page retirement removes that slot with a zero-sized region while the guest
/// runs: `soma-kvm/src/x86_64/launch_page.rs` `unregister`, reached from `retire_launch_page`
/// during teardown and immediately after the guest consumes the page.
/// The snapshot state groups are reserved for the restore slice and admitted only at startup;
/// a Generation-building capture run keeps the startup filter for its whole life.
pub const IOCTL_RULES: &[IoctlRule] = &[
    rule("KVM_GET_API_VERSION", io(0x00), StartupOnly, KVM),
    rule("KVM_CREATE_VM", io(0x01), StartupOnly, KVM),
    rule("KVM_CHECK_EXTENSION", io(0x03), StartupOnly, KVM),
    rule("KVM_GET_VCPU_MMAP_SIZE", io(0x04), StartupOnly, KVM),
    rule("KVM_GET_SUPPORTED_CPUID", iowr(0x05, 8), StartupOnly, KVM),
    rule("KVM_CREATE_VCPU", io(0x41), StartupOnly, KVM),
    rule("KVM_SET_USER_MEMORY_REGION", iow(0x46, 32), Both, KVM),
    rule("KVM_SET_TSS_ADDR", io(0x47), StartupOnly, KVM),
    rule("KVM_CREATE_IRQCHIP", io(0x60), StartupOnly, KVM),
    rule("KVM_GET_IRQCHIP", iowr(0x62, 520), StartupOnly, SNAPSHOT),
    rule("KVM_SET_IRQCHIP", ior(0x63, 520), StartupOnly, SNAPSHOT),
    rule("KVM_SET_GSI_ROUTING", iow(0x6a, 8), StartupOnly, SNAPSHOT),
    rule("KVM_IRQFD", iow(0x76, 32), Both, KVM),
    rule("KVM_CREATE_PIT2", iow(0x77, 64), StartupOnly, KVM),
    rule("KVM_IOEVENTFD", iow(0x79, 64), Both, VIRTIO),
    rule("KVM_GET_CLOCK", ior(0x7c, 48), StartupOnly, SNAPSHOT),
    rule("KVM_SET_CLOCK", iow(0x7d, 48), StartupOnly, SNAPSHOT),
    rule("KVM_RUN", io(0x80), Both, KVM),
    rule("KVM_GET_REGS", ior(0x81, 144), StartupOnly, SNAPSHOT),
    rule("KVM_SET_REGS", iow(0x82, 144), StartupOnly, KVM),
    rule("KVM_GET_SREGS", ior(0x83, 312), StartupOnly, KVM),
    rule("KVM_SET_SREGS", iow(0x84, 312), StartupOnly, KVM),
    rule("KVM_GET_MSRS", iowr(0x88, 8), StartupOnly, SNAPSHOT),
    rule("KVM_SET_MSRS", iow(0x89, 8), StartupOnly, SNAPSHOT),
    rule("KVM_SET_SIGNAL_MASK", iow(0x8b, 4), StartupOnly, KVM),
    rule("KVM_GET_FPU", ior(0x8c, 416), StartupOnly, SNAPSHOT),
    rule("KVM_SET_FPU", iow(0x8d, 416), StartupOnly, SNAPSHOT),
    rule("KVM_GET_LAPIC", ior(0x8e, 1024), StartupOnly, SNAPSHOT),
    rule("KVM_SET_LAPIC", iow(0x8f, 1024), StartupOnly, SNAPSHOT),
    rule("KVM_SET_CPUID2", iow(0x90, 8), StartupOnly, KVM),
    rule("KVM_GET_MP_STATE", ior(0x98, 4), StartupOnly, SNAPSHOT),
    rule("KVM_SET_MP_STATE", iow(0x99, 4), StartupOnly, SNAPSHOT),
    rule("KVM_GET_VCPU_EVENTS", ior(0x9f, 64), StartupOnly, SNAPSHOT),
    rule("KVM_SET_VCPU_EVENTS", iow(0xa0, 64), StartupOnly, SNAPSHOT),
    rule("KVM_GET_PIT2", ior(0x9f, 112), StartupOnly, SNAPSHOT),
    rule("KVM_SET_PIT2", iow(0xa0, 112), StartupOnly, SNAPSHOT),
    rule("KVM_GET_XSAVE", ior(0xa4, 4096), StartupOnly, SNAPSHOT),
    rule("KVM_SET_XSAVE", iow(0xa5, 4096), StartupOnly, SNAPSHOT),
    rule("KVM_GET_XCRS", ior(0xa6, 392), StartupOnly, SNAPSHOT),
    rule("KVM_SET_XCRS", iow(0xa7, 392), StartupOnly, SNAPSHOT),
    rule(
        "KVM_GET_NESTED_STATE",
        iowr(0xbe, 128),
        StartupOnly,
        SNAPSHOT,
    ),
    rule(
        "KVM_SET_NESTED_STATE",
        iow(0xbf, 128),
        StartupOnly,
        SNAPSHOT,
    ),
    rule("FIONBIO", 0x5421, Both, RUST),
];

/// `TUNSETIFF`, listed only so tests can prove it is rejected.
pub const TUNSETIFF: u32 = 0x4004_54CA;

/// The documented steady-state request set; the tests prove the table matches it exactly.
pub const STEADY_REQUESTS: &[&str] = &[
    "KVM_SET_USER_MEMORY_REGION",
    "KVM_IRQFD",
    "KVM_IOEVENTFD",
    "KVM_RUN",
    "FIONBIO",
];

/// Request values admitted in `phase`, in table order.
#[must_use]
pub fn requests_for(phase: Phase) -> Vec<u32> {
    IOCTL_RULES
        .iter()
        .filter(|rule| phase == Phase::Startup || rule.phases == Both)
        .map(|rule| rule.request)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Both, IOCTL_RULES, IoctlPhases};

    /// The probe binary cannot reach this table, so it carries the literal request value.
    /// This pins that literal, and the steady-state classification the live probe depends on.
    #[test]
    fn the_launch_page_retirement_request_matches_the_probe_literal() {
        let rule = IOCTL_RULES
            .iter()
            .find(|rule| rule.name == "KVM_SET_USER_MEMORY_REGION")
            .expect("the table admits the launch-page retirement request");
        assert_eq!(
            rule.request, 0x4020_AE46,
            "jail-probe.rs carries this literal"
        );
        assert_eq!(
            rule.phases, Both,
            "launch_page.rs `unregister` issues this while the guest runs, so a startup-only \
             classification would make the jail kill the VMM at teardown",
        );
        let _ = IoctlPhases::StartupOnly;
    }
}
