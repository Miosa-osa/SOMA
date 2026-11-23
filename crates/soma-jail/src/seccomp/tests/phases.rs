//! Request sign extension and the exact difference between the startup and steady phases.

use super::*;

#[test]
fn ioctl_musl_sign_extension_preserves_the_exact_request_and_phase() {
    let signed = |request: u32| u64::from(request) | (u64::from(u32::MAX) << 32);
    let args = |request| [0, request, 0, 0, 0, 0];
    let cpuid = 0xc008_ae05;
    let set_irqchip = 0x8208_ae63;
    assert_eq!(
        run(Phase::Startup, nr("ioctl"), args(signed(cpuid))),
        bpf::RET_ALLOW
    );
    assert_eq!(
        run(Phase::SteadyState, nr("ioctl"), args(signed(cpuid))),
        bpf::RET_ALLOW
    );
    assert_eq!(
        run(Phase::Startup, nr("ioctl"), args(signed(set_irqchip))),
        bpf::RET_ALLOW
    );
    assert_eq!(
        run(Phase::SteadyState, nr("ioctl"), args(signed(set_irqchip))),
        bpf::RET_KILL_PROCESS
    );
    assert_eq!(
        run(Phase::Startup, nr("ioctl"), args(signed(0xc008_ae06))),
        bpf::RET_KILL_PROCESS
    );
    assert_eq!(
        run(Phase::Startup, nr("ioctl"), args(signed(0x0000_ae03))),
        bpf::RET_KILL_PROCESS
    );
    assert_eq!(
        run(
            Phase::Startup,
            nr("ioctl"),
            args(u64::from(cpuid) | (1 << 32))
        ),
        bpf::RET_KILL_PROCESS
    );
}

#[test]
fn phase_difference_is_exactly_the_documented_set() {
    let mut computed = startup_only_syscalls();
    computed.sort_unstable();
    let mut documented = policy::STARTUP_ONLY.to_vec();
    documented.sort_unstable();
    assert_eq!(computed, documented);
    let mut steady: Vec<&str> = ioctls::IOCTL_RULES
        .iter()
        .filter(|rule| rule.phases == ioctls::IoctlPhases::Both)
        .map(|rule| rule.name)
        .collect();
    steady.sort_unstable();
    let mut documented = ioctls::STEADY_REQUESTS.to_vec();
    documented.sort_unstable();
    assert_eq!(steady, documented);
    let tightened: Vec<&str> = policy::RULES
        .iter()
        .filter(|rule| rule.steady.is_some_and(|steady| steady != rule.startup))
        .map(|rule| rule.name)
        .collect();
    assert_eq!(tightened, ["mmap", "mprotect", "prctl"]);
    assert!(
        policy::RULES
            .iter()
            .any(|rule| rule.startup == ArgCheck::IoctlAllowlist)
    );
}
