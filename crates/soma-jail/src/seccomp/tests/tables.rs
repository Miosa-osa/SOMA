//! Table invariants: uniqueness, ordering, the denial surface, and agreement with `libc` and
//! `kvm-bindings` on the production target.

use super::{nr, run};
use crate::seccomp::{Phase, bpf, denied, ioctls, policy};

const KILL: u32 = bpf::RET_KILL_PROCESS;
const TUNSETIFF: u32 = ioctls::TUNSETIFF;

#[test]
fn never_allowed_syscalls_are_absent_and_killed() {
    for (name, number) in denied::NEVER_ALLOWED {
        assert!(
            !policy::RULES.iter().any(|rule| rule.nr == *number),
            "{name} is in the table"
        );
        for phase in [Phase::Startup, Phase::SteadyState] {
            assert_eq!(run(phase, *number, [0; 6]), KILL, "{name}");
        }
    }
}

#[test]
fn tables_are_unique_and_ordered() {
    let numbers: Vec<u32> = policy::RULES.iter().map(|rule| rule.nr).collect();
    let mut sorted = numbers.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        numbers, sorted,
        "syscall table must be ascending and unique"
    );
    let mut requests: Vec<u32> = ioctls::IOCTL_RULES
        .iter()
        .map(|rule| rule.request)
        .collect();
    let before = requests.len();
    requests.sort_unstable();
    requests.dedup();
    assert_eq!(requests.len(), before, "ioctl requests must be unique");
    assert!(
        !ioctls::IOCTL_RULES
            .iter()
            .any(|rule| rule.request == TUNSETIFF)
    );
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn numbers_match_libc_and_kvm_bindings() {
    use std::mem::size_of;

    use kvm_bindings::{
        kvm_clock_data, kvm_cpuid2, kvm_fpu, kvm_irq_routing, kvm_irqchip, kvm_irqfd,
        kvm_lapic_state, kvm_msrs, kvm_nested_state, kvm_pit_config, kvm_pit_state2, kvm_regs,
        kvm_signal_mask, kvm_sregs, kvm_userspace_memory_region, kvm_vcpu_events, kvm_xcrs,
        kvm_xsave,
    };

    let sizes: [(&str, usize); 18] = [
        ("KVM_GET_SUPPORTED_CPUID", size_of::<kvm_cpuid2>()),
        (
            "KVM_SET_USER_MEMORY_REGION",
            size_of::<kvm_userspace_memory_region>(),
        ),
        ("KVM_GET_IRQCHIP", size_of::<kvm_irqchip>()),
        ("KVM_SET_GSI_ROUTING", size_of::<kvm_irq_routing>()),
        ("KVM_IRQFD", size_of::<kvm_irqfd>()),
        ("KVM_CREATE_PIT2", size_of::<kvm_pit_config>()),
        ("KVM_GET_CLOCK", size_of::<kvm_clock_data>()),
        ("KVM_GET_REGS", size_of::<kvm_regs>()),
        ("KVM_GET_SREGS", size_of::<kvm_sregs>()),
        ("KVM_GET_MSRS", size_of::<kvm_msrs>()),
        ("KVM_SET_SIGNAL_MASK", size_of::<kvm_signal_mask>()),
        ("KVM_GET_FPU", size_of::<kvm_fpu>()),
        ("KVM_GET_LAPIC", size_of::<kvm_lapic_state>()),
        ("KVM_GET_VCPU_EVENTS", size_of::<kvm_vcpu_events>()),
        ("KVM_GET_PIT2", size_of::<kvm_pit_state2>()),
        ("KVM_GET_XSAVE", size_of::<kvm_xsave>()),
        ("KVM_GET_XCRS", size_of::<kvm_xcrs>()),
        ("KVM_GET_NESTED_STATE", size_of::<kvm_nested_state>()),
    ];
    for (name, size) in sizes {
        let rule = ioctls::IOCTL_RULES
            .iter()
            .find(|rule| rule.name == name)
            .expect(name);
        let encoded = usize::try_from((rule.request >> 16) & 0x3fff).expect("size");
        assert_eq!(encoded, size, "{name}");
    }
    let expected: [(&str, libc::c_long); 13] = [
        ("read", libc::SYS_read),
        ("ioctl", libc::SYS_ioctl),
        ("clone", libc::SYS_clone),
        ("clone3", libc::SYS_clone3),
        ("execveat", libc::SYS_execveat),
        ("seccomp", libc::SYS_seccomp),
        ("prctl", libc::SYS_prctl),
        ("rseq", libc::SYS_rseq),
        ("epoll_pwait2", libc::SYS_epoll_pwait2),
        ("getrandom", libc::SYS_getrandom),
        ("tgkill", libc::SYS_tgkill),
        ("tkill", libc::SYS_tkill),
        ("set_robust_list", libc::SYS_set_robust_list),
    ];
    for (name, number) in expected {
        assert_eq!(libc::c_long::from(nr(name)), number, "{name}");
    }
    for (name, number) in denied::NEVER_ALLOWED {
        let known = match *name {
            "socket" => libc::SYS_socket,
            "dup3" => libc::SYS_dup3,
            "chroot" => libc::SYS_chroot,
            "execve" => libc::SYS_execve,
            "mount" => libc::SYS_mount,
            "unshare" => libc::SYS_unshare,
            "setresuid" => libc::SYS_setresuid,
            "ptrace" => libc::SYS_ptrace,
            "bpf" => libc::SYS_bpf,
            "pidfd_open" => libc::SYS_pidfd_open,
            _ => continue,
        };
        assert_eq!(libc::c_long::from(*number), known, "{name}");
    }
}
