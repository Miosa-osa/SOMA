//! The per-phase syscall allowlist for the `x86_64` VMM.
//!
//! Every entry names its provenance.
//! `Measured` entries were observed in an `strace -f` of the static musl `jail-probe` child
//! inside the live container, in `soma-kvm` code, or in the Rust runtime; `Reserved` entries
//! exist for a documented future path and were not yet observed.
//! Everything absent is killed; `denied.rs` names the surface the tests prove absent.

use super::ArgCheck;

/// Where an allowlist entry came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Provenance {
    /// Observed in current code or a retained trace.
    Measured(Source),
    /// Needed by a documented future path and not yet observed.
    Reserved(Need),
}

/// The observation that justifies a measured entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
    /// `strace -f` of the static musl `jail-probe` child, retained in the live evidence.
    ProbeTrace,
    /// `crates/soma-kvm/src/x86_64`, `machine.rs`, and `linux.rs`.
    SomaKvm,
    /// The Rust standard library runtime on Linux.
    RustRuntime,
    /// The launcher itself, immediately before `execveat`.
    Launcher,
}

/// The future path that justifies a reserved entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Need {
    DiskBackend,
    VirtioDevices,
    /// Prepared-worker claim transfers descriptors with `SCM_RIGHTS`.
    DescriptorTransfer,
    EventLoop,
    SnapshotRestore,
    /// A glibc-linked VMM; the musl probe never issues these.
    GlibcRuntime,
    /// `Vec` growth and musl `mallocng` use `mremap` and `madvise` on large blocks.
    Allocator,
}

/// One syscall entry; `steady` is `None` for a startup-only syscall.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SyscallRule {
    pub name: &'static str,
    pub nr: u32,
    pub provenance: Provenance,
    pub startup: ArgCheck,
    pub steady: Option<ArgCheck>,
}

const fn both(name: &'static str, nr: u32, provenance: Provenance, check: ArgCheck) -> SyscallRule {
    SyscallRule {
        name,
        nr,
        provenance,
        startup: check,
        steady: Some(check),
    }
}

const fn startup(
    name: &'static str,
    nr: u32,
    provenance: Provenance,
    check: ArgCheck,
) -> SyscallRule {
    SyscallRule {
        name,
        nr,
        provenance,
        startup: check,
        steady: None,
    }
}

const fn tightened(
    name: &'static str,
    nr: u32,
    provenance: Provenance,
    startup: ArgCheck,
    steady: ArgCheck,
) -> SyscallRule {
    SyscallRule {
        name,
        nr,
        provenance,
        startup,
        steady: Some(steady),
    }
}

const PROBE: Provenance = Provenance::Measured(Source::ProbeTrace);
const KVM: Provenance = Provenance::Measured(Source::SomaKvm);
const RUST: Provenance = Provenance::Measured(Source::RustRuntime);
const LAUNCHER: Provenance = Provenance::Measured(Source::Launcher);
const DISK: Provenance = Provenance::Reserved(Need::DiskBackend);
const VIRTIO: Provenance = Provenance::Reserved(Need::VirtioDevices);
const TRANSFER: Provenance = Provenance::Reserved(Need::DescriptorTransfer);
const LOOP: Provenance = Provenance::Reserved(Need::EventLoop);
const GLIBC: Provenance = Provenance::Reserved(Need::GlibcRuntime);
const ALLOC: Provenance = Provenance::Reserved(Need::Allocator);

/// `CLONE_THREAD`.
pub const CLONE_THREAD: u32 = 0x0001_0000;
/// Every namespace flag `clone` could carry: `CLONE_NEWNS`, `CLONE_NEWCGROUP`,
/// `CLONE_NEWUTS`, `CLONE_NEWIPC`, `CLONE_NEWUSER`, `CLONE_NEWPID`, and `CLONE_NEWNET`.
pub const CLONE_NAMESPACE_MASK: u32 =
    0x0002_0000 | 0x0200_0000 | 0x0400_0000 | 0x0800_0000 | 0x1000_0000 | 0x2000_0000 | 0x4000_0000;
/// `PROT_EXEC`.
pub const PROT_EXEC: u32 = 0x4;
/// `ENOSYS`, returned for `clone3` so glibc falls back to inspectable `clone`.
pub const ENOSYS: u16 = 38;

/// `PR_SET_NAME`, `PR_GET_NAME`, and `PR_SET_NO_NEW_PRIVS`; the death signal and dumpable
/// state were fixed by the launcher before the filter existed.
const PRCTL_STARTUP: &[u32] = &[15, 16, 38];
const PRCTL_STEADY: &[u32] = &[15, 16];
/// `F_GETFD`, `F_SETFD`, `F_GETFL`, `F_SETFL`, and `F_DUPFD_CLOEXEC`.
const FCNTL_STARTUP: &[u32] = &[1, 2, 3, 4, 1030];

const THREAD_CLONE: ArgCheck = ArgCheck::Flags {
    index: 0,
    forbidden: CLONE_NAMESPACE_MASK,
    required: CLONE_THREAD,
};
const NO_EXEC_MAPPING: ArgCheck = ArgCheck::Flags {
    index: 2,
    forbidden: PROT_EXEC,
    required: 0,
};

/// The complete rule table, in `x86_64` syscall-number order.
///
/// musl issues the legacy `open`, `stat`, and `tkill` where glibc issues `openat`,
/// `newfstatat` or `statx`, and `tgkill`; both link models are admitted so the VMM's link
/// model is not decided by the filter.
pub const RULES: &[SyscallRule] = &[
    both("read", 0, KVM, ArgCheck::Any),
    both("write", 1, KVM, ArgCheck::Any),
    startup("open", 2, PROBE, ArgCheck::Any),
    both("close", 3, PROBE, ArgCheck::Any),
    startup("stat", 4, PROBE, ArgCheck::Any),
    both("fstat", 5, PROBE, ArgCheck::Any),
    startup("poll", 7, PROBE, ArgCheck::Any),
    both("lseek", 8, DISK, ArgCheck::Any),
    tightened("mmap", 9, PROBE, ArgCheck::Any, NO_EXEC_MAPPING),
    tightened("mprotect", 10, PROBE, ArgCheck::Any, NO_EXEC_MAPPING),
    both("munmap", 11, PROBE, ArgCheck::Any),
    both("brk", 12, PROBE, ArgCheck::Any),
    startup("rt_sigaction", 13, PROBE, ArgCheck::Any),
    both("rt_sigprocmask", 14, PROBE, ArgCheck::Any),
    both("rt_sigreturn", 15, KVM, ArgCheck::Any),
    tightened(
        "ioctl",
        16,
        KVM,
        ArgCheck::IoctlAllowlist,
        ArgCheck::IoctlAllowlist,
    ),
    both("pread64", 17, DISK, ArgCheck::Any),
    both("pwrite64", 18, DISK, ArgCheck::Any),
    both("readv", 19, VIRTIO, ArgCheck::Any),
    both("writev", 20, VIRTIO, ArgCheck::Any),
    both("sched_yield", 24, LOOP, ArgCheck::Any),
    both("mremap", 25, ALLOC, ArgCheck::Any),
    both("madvise", 28, ALLOC, ArgCheck::Any),
    both("nanosleep", 35, LOOP, ArgCheck::Any),
    both("getpid", 39, KVM, ArgCheck::Any),
    both("sendto", 44, PROBE, ArgCheck::Any),
    both("recvfrom", 45, PROBE, ArgCheck::Any),
    both("sendmsg", 46, TRANSFER, ArgCheck::Any),
    both("recvmsg", 47, TRANSFER, ArgCheck::Any),
    both("clone", 56, PROBE, THREAD_CLONE),
    both("exit", 60, KVM, ArgCheck::Any),
    startup(
        "fcntl",
        72,
        PROBE,
        ArgCheck::ArgEqualsAny {
            index: 1,
            values: FCNTL_STARTUP,
        },
    ),
    both("fsync", 74, DISK, ArgCheck::Any),
    both("fdatasync", 75, DISK, ArgCheck::Any),
    startup("getrlimit", 97, GLIBC, ArgCheck::Any),
    startup("getuid", 102, PROBE, ArgCheck::Any),
    startup("getgid", 104, PROBE, ArgCheck::Any),
    startup("geteuid", 107, PROBE, ArgCheck::Any),
    startup("getegid", 108, PROBE, ArgCheck::Any),
    both("sigaltstack", 131, PROBE, ArgCheck::Any),
    tightened(
        "prctl",
        157,
        PROBE,
        ArgCheck::ArgEqualsAny {
            index: 0,
            values: PRCTL_STARTUP,
        },
        ArgCheck::ArgEqualsAny {
            index: 0,
            values: PRCTL_STEADY,
        },
    ),
    startup("arch_prctl", 158, PROBE, ArgCheck::Any),
    both("gettid", 186, PROBE, ArgCheck::Any),
    both("tkill", 200, KVM, ArgCheck::Any),
    both("futex", 202, PROBE, ArgCheck::Any),
    startup("getdents64", 217, PROBE, ArgCheck::Any),
    startup("set_tid_address", 218, PROBE, ArgCheck::Any),
    both("restart_syscall", 219, LOOP, ArgCheck::Any),
    both("clock_gettime", 228, LOOP, ArgCheck::Any),
    both("clock_nanosleep", 230, LOOP, ArgCheck::Any),
    both("exit_group", 231, PROBE, ArgCheck::Any),
    both("epoll_wait", 232, LOOP, ArgCheck::Any),
    both("epoll_ctl", 233, LOOP, ArgCheck::Any),
    both("tgkill", 234, KVM, ArgCheck::Any),
    startup("openat", 257, GLIBC, ArgCheck::Any),
    startup("newfstatat", 262, GLIBC, ArgCheck::Any),
    both("ppoll", 271, LOOP, ArgCheck::Any),
    both("set_robust_list", 273, GLIBC, ArgCheck::Any),
    both("epoll_pwait", 281, LOOP, ArgCheck::Any),
    startup("timerfd_create", 283, LOOP, ArgCheck::Any),
    both("fallocate", 285, DISK, ArgCheck::Any),
    both("timerfd_settime", 286, LOOP, ArgCheck::Any),
    both("timerfd_gettime", 287, LOOP, ArgCheck::Any),
    startup("eventfd2", 290, KVM, ArgCheck::Any),
    startup("epoll_create1", 291, LOOP, ArgCheck::Any),
    both("preadv", 295, DISK, ArgCheck::Any),
    both("pwritev", 296, DISK, ArgCheck::Any),
    startup("prlimit64", 302, PROBE, ArgCheck::Any),
    startup("seccomp", 317, PROBE, ArgCheck::Any),
    both("getrandom", 318, RUST, ArgCheck::Any),
    startup("execveat", 322, LAUNCHER, ArgCheck::Any),
    startup("statx", 332, GLIBC, ArgCheck::Any),
    both("rseq", 334, GLIBC, ArgCheck::Any),
    both("clone3", 435, GLIBC, ArgCheck::Errno(ENOSYS)),
    both("epoll_pwait2", 441, LOOP, ArgCheck::Any),
];

/// The documented startup-only set; the tests prove `RULES` matches it exactly.
pub const STARTUP_ONLY: &[&str] = &[
    "open",
    "stat",
    "poll",
    "rt_sigaction",
    "fcntl",
    "getrlimit",
    "getuid",
    "getgid",
    "geteuid",
    "getegid",
    "arch_prctl",
    "getdents64",
    "set_tid_address",
    "openat",
    "newfstatat",
    "timerfd_create",
    "eventfd2",
    "epoll_create1",
    "prlimit64",
    "seccomp",
    "execveat",
    "statx",
];
