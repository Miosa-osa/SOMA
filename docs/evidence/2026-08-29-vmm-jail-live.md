# VMM jail live acceptance run - 2026-08-29

## Evidence boundary

This result proves that the `soma-jail` launcher, running as root inside a privileged Ubuntu 24.04 container on an Ubuntu 24.04 x86_64 host, constrains the static `jail-probe` stand-in exactly as the [VMM jail profile](../research/vmm-jail-profile.md) requires: only the manifest descriptors are visible, the child runs as an ephemeral uid and gid inside six fresh namespaces, its root is an empty read-only tmpfs with no procfs or sysfs, its cgroup v2 leaf reads back the written limits, a forbidden syscall or ioctl kills it with `SIGSYS` and is recorded in the evidence, the steady-state filter drops setup-only syscalls and ioctls while threads keep working, pids and memory exhaustion stay inside the leaf, a stuck child dies through its pidfd, the pidfd never targets a reused PID, the child dies with its launching thread and with its launcher process, a crashed launcher's leaf is recovered, injected descriptors fail closed before seccomp, and every run ends with zero residual resources.

It does not prove anything about the real `soma-vmm` binary, a transferred TAP endpoint, `io.max`, snapshot ioctls, a stuck `KVM_RUN`, prepared workers, or any latency objective.
The probe is a musl static-pie Rust binary, so the measured syscall inventory is that of musl plus the Rust runtime; a glibc-linked VMM would issue the reserved `openat`, `newfstatat`, `statx`, `getrlimit`, `set_robust_list`, `rseq`, and `clone3` entries instead, which this run could not observe.

## Identities

- SOMA Git revision: branch `feat/vmm-jail`, commits `fa948a7` (launcher) and `57c99e0` (live suite) rebased onto `origin/main` at `b1bb606`; the run below was repeated on that rebased tree.
- Host kernel: `Linux 7.0.0-30-generic` x86_64.
- Host distribution: Ubuntu 24.04.4 LTS.
- Host user namespaces: `unshare -Ur` fails with `EPERM` on this host because Ubuntu's AppArmor restriction on unprivileged user namespaces is enabled, so the suite runs inside a privileged container where the launcher is root and unconfined; the jail itself is what the tests constrain.
- Container runtime: Docker 29.3.0, cgroup v2 with the systemd driver, `docker run --rm --privileged --cgroupns=private`.
- Container image: `ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517`, `/etc/os-release` reports `ubuntu 24.04`; the same digest pins the kernel builder image.
- Container cgroup preparation: every process is moved from the cgroup namespace root into `init`, `+cpu +memory +pids +io` is written to the root's `cgroup.subtree_control`, and the delegated subtree `/sys/fs/cgroup/soma-jail` reads back `cpu io memory pids`.
- Devices: `/dev/kvm` is character device 10:232 inside the container and is the only device the tests transfer.
- Rust toolchain: `1.98.0 (88d9e12ae 2026-08-18)`, target `x86_64-unknown-linux-musl` for both the probe and the test binary.
- Probe: `target/x86_64-unknown-linux-musl/debug/jail-probe`, 6,929,080 bytes, SHA-256 `000f2dc4b2ec9878035a3e189bdbba88a4bc100978be0f38d0930deb66fa0877`, `file` reports `static-pie linked`; this is a debug build and the digest is a local measurement.
- Jail specification used by every test: uid and gid 60001, `memory.max` 64 MiB (32 MiB for the OOM test), `memory.swap.max` 0, `memory.oom.group` 1, `cpu.max` `100000 100000`, `pids.max` 8, `RLIMIT_NOFILE` 16, `RLIMIT_NPROC` 64, `RLIMIT_FSIZE` 1 GiB, `RLIMIT_CORE` 0, manifest `control` or `kvm,control`.
- Startup filter: 222 classic BPF instructions, FNV-1a fingerprint `0x40b7c33a9001c79b`; steady-state filter: 135 instructions, fingerprint `0xe748c586d5877538`; both pinned by `programs_assemble_deterministically`.

## Invocation

```sh
./scripts/jail-live-tests.sh
```

The script builds the probe and the `jail_live` test binary for the musl target, refuses a binary that is not statically linked, pulls the digest-pinned image, mounts the worktree read-only, prepares the cgroup subtree, and runs `jail_live --ignored --test-threads=1 --skip helper_` as root.
Each test calls `require()` first and fails with the complete list of missing prerequisites; nothing skips or passes silently.

## Result

```text
kernel: 7.0.0-30-generic
container identity: uid=0(root) gid=0(root) groups=0(root)
image os: ubuntu 24.04
delegated controllers: cpu io memory pids
crw-rw---- 1 root 993 10, 232 Aug 29 13:04 /dev/kvm

running 15 tests
test containment::cgroup_limits_read_back_and_contain_the_child ... ok
test containment::child_runs_as_the_ephemeral_identity_in_six_fresh_namespaces ... ok
test containment::forbidden_syscalls_are_recorded_seccomp_kills ... ok
test containment::kvm_version_is_admitted_while_tunsetiff_is_killed ... ok
test containment::root_is_empty_read_only_without_procfs_or_sysfs ... ok
test containment::sealed_table_hides_an_injected_descriptor ... ok
test containment::steady_state_keeps_threads_but_drops_setup_syscalls_and_ioctls ... ok
test failure::an_existing_leaf_fails_closed_and_is_never_reused ... ok
test failure::launcher_process_death_kills_the_child_and_recovery_releases ... ok
test failure::launching_thread_exit_kills_the_child ... ok
test failure::memory_max_oom_kills_the_whole_group ... ok
test failure::pidfd_identity_outlives_the_numeric_pid ... ok
test failure::pids_max_exhaustion_is_contained ... ok
test failure::stuck_child_is_killed_through_the_pidfd_at_reconcile ... ok
test failure::wrong_descriptor_kinds_fail_closed_before_seccomp ... ok

test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.71s
```

The filtered-out test is `helper_launch_and_sleep_until_killed`, which only the launcher-death test spawns as a separate process and which fails if invoked directly.
The same fifteen tests passed a second time under `strace -f` 6.8 inside the same image, which produced the syscall inventory below.

## What each test proved

- `sealed_table_hides_an_injected_descriptor`: with a stray non-close-on-exec descriptor open in the launcher, the probe verified every slot by `fstat`, `/proc/<pid>/fd` from the launcher side listed exactly `0 1 2 3 4` for the `kvm,control` manifest, and the evidence recorded five descriptors.
- `child_runs_as_the_ephemeral_identity_in_six_fresh_namespaces`: the probe reported uid, euid, gid, and egid 60001 and PID 1; `/proc/<pid>/status` showed uid and gid 60001, `NoNewPrivs 1`, `Seccomp 2`, and zero effective and permitted capabilities; all six namespace inodes differed from the launcher's; the network namespace held only `lo`.
- `root_is_empty_read_only_without_procfs_or_sysfs`: `/` had zero entries, creating a file failed, `/proc` and `/sys` did not exist, and `openat(O_CREAT)` returned `EROFS`.
- `cgroup_limits_read_back_and_contain_the_child`: `memory.max`, `memory.swap.max`, `memory.oom.group`, `cpu.max`, and `pids.max` read back exactly, and `cgroup.procs` listed the child.
- `forbidden_syscalls_are_recorded_seccomp_kills`: `socket(2)` and `execve(2)` each killed the process with `SIGSYS`; `wait` returned the signal, `reconcile` returned `Released`, and the evidence carried the `SIGSYS` exit.
- `kvm_version_is_admitted_while_tunsetiff_is_killed`: `KVM_GET_API_VERSION` on the transferred `/dev/kvm` returned 12 under the startup filter and `TUNSETIFF` on the same descriptor was a `SIGSYS` kill.
- `steady_state_keeps_threads_but_drops_setup_syscalls_and_ioctls`: after the probe stacked the steady-state filter, two threads still spawned, `KVM_GET_API_VERSION` became a kill, and in a second jail `openat(O_CREAT)` became a kill.
- `pids_max_exhaustion_is_contained`: asking for 32 threads under `pids.max 8` spawned fewer than eight and then failed with `EAGAIN`; the launcher and the probe stayed healthy and the jail exited cleanly.
- `memory_max_oom_kills_the_whole_group`: touching 96 MiB under `memory.max` 32 MiB ended with `SIGKILL` and `memory.events` recorded at least one `oom_kill`.
- `stuck_child_is_killed_through_the_pidfd_at_reconcile`: a silent child timed out on `wait`, `reconcile` killed it through the pidfd, and the evidence recorded `SIGKILL`.
- `pidfd_identity_outlives_the_numeric_pid`: signal 0 through the pidfd succeeded while the child lived and returned `Gone` after it exited; the numeric PID is never used to signal, but PID reuse itself was not forced.
- `launching_thread_exit_kills_the_child`: a jail launched from a scoped thread died with `SIGKILL` when that thread ended, because `PR_SET_PDEATHSIG` binds to the creating thread.
- `launcher_process_death_kills_the_child_and_recovery_releases`: a separate launcher process was `SIGKILL`ed; the test process, as child subreaper, reaped the orphaned probe with `CLD_KILLED` and `SIGKILL`, then `JailLedger::recover` released the leaf and jail root from the record alone.
- `wrong_descriptor_kinds_fail_closed_before_seccomp`: a regular file in the control slot failed with `Verify(Kind { slot: 3, found: RegularFile })` and a `SOCK_STREAM` socket failed with `Verify(NotSeqpacket { slot: 3 })`, both with cleanup `Released` and nothing left behind.
- `an_existing_leaf_fails_closed_and_is_never_reused`: a pre-existing leaf produced `Cgroup(AlreadyExists)` and the launcher did not remove a leaf it did not own.

## Measured syscall inventory

The inventory comes from the retained `strace -f` pass over all fifteen tests, attributing each task to the probe that executed or cloned it; the fifteen probe processes and their nine threads issued the following calls after `execveat`.

Startup phase, all probe tasks: `arch_prctl`, `brk`, `clone`, `close`, `exit_group`, `fcntl` (`F_GETFD` from musl's `fstat` wrapper after `EBADF`), `fstat`, `futex`, `getdents64`, `getegid`, `geteuid`, `getgid`, `getpid`, `gettid`, `getuid`, `ioctl`, `mmap`, `mprotect`, `munmap`, `open`, `poll`, `prctl` (`PR_SET_NO_NEW_PRIVS`), `prlimit64`, `recvfrom`, `rt_sigaction`, `rt_sigprocmask`, `sendto`, `set_tid_address`, `sigaltstack`, `stat`, plus the killed attempts `socket` and `execve`.

Steady phase, all probe tasks: `clone`, `futex`, `gettid`, `mmap`, `mprotect`, `munmap`, `recvfrom`, `rt_sigprocmask`, `sendto`, `sigaltstack`, plus the killed attempts `ioctl(KVM_GET_API_VERSION)` and `open`.

Probe threads only: `futex`, `gettid`, `mmap`, `mprotect`, `rt_sigprocmask`, `sigaltstack`.

ioctl requests observed: `KVM_GET_API_VERSION` twice, `TUNSETIFF` once, the latter killed.

Every observed syscall is either in the table or was killed on purpose.
Two musl facts were only learned from this trace and changed the table: musl issues the legacy `open` and `stat` syscalls where glibc issues `openat` and `newfstatat` or `statx`, and every new Rust thread calls `gettid` first.

## Allowlist as shipped

Startup-only syscalls, removed by the steady-state filter: `open`, `stat`, `poll`, `rt_sigaction`, `fcntl`, `getrlimit`, `getuid`, `getgid`, `geteuid`, `getegid`, `arch_prctl`, `getdents64`, `set_tid_address`, `openat`, `newfstatat`, `timerfd_create`, `eventfd2`, `epoll_create1`, `prlimit64`, `seccomp`, `execveat`, `statx`.

Tightened in steady state: `mmap` and `mprotect` reject `PROT_EXEC`; `prctl` keeps only `PR_SET_NAME` and `PR_GET_NAME`.

Admitted in both phases: `read`, `write`, `close`, `fstat`, `lseek`, `mmap`, `mprotect`, `munmap`, `brk`, `rt_sigprocmask`, `rt_sigreturn`, `ioctl` (request allowlist), `pread64`, `pwrite64`, `readv`, `writev`, `sched_yield`, `mremap`, `madvise`, `nanosleep`, `getpid`, `sendto`, `recvfrom`, `sendmsg`, `recvmsg`, `clone` (`CLONE_THREAD` required, every namespace flag forbidden), `exit`, `fsync`, `fdatasync`, `sigaltstack`, `prctl`, `gettid`, `tkill`, `futex`, `restart_syscall`, `clock_gettime`, `clock_nanosleep`, `exit_group`, `epoll_wait`, `epoll_ctl`, `tgkill`, `ppoll`, `set_robust_list`, `epoll_pwait`, `fallocate`, `timerfd_settime`, `timerfd_gettime`, `preadv`, `pwritev`, `getrandom`, `rseq`, `clone3` (fails with `ENOSYS`), `epoll_pwait2`.

Measured provenance: probe trace covers `open`, `close`, `stat`, `fstat`, `poll`, `mmap`, `mprotect`, `munmap`, `brk`, `rt_sigaction`, `rt_sigprocmask`, `sendto`, `recvfrom`, `clone`, `fcntl`, `getuid`, `getgid`, `geteuid`, `getegid`, `sigaltstack`, `prctl`, `arch_prctl`, `gettid`, `futex`, `getdents64`, `set_tid_address`, `exit_group`, `prlimit64`, and `seccomp`; `soma-kvm` code covers `read`, `write`, `rt_sigreturn`, `ioctl`, `getpid`, `exit`, `tkill`, `tgkill`, and `eventfd2`; the Rust runtime covers `getrandom`; the launcher covers `execveat`.

Reserved provenance, not yet observed: disk backend (`lseek`, `pread64`, `pwrite64`, `fsync`, `fdatasync`, `fallocate`, `preadv`, `pwritev`), virtio devices (`readv`, `writev`), descriptor transfer (`sendmsg`, `recvmsg`), event loop (`sched_yield`, `nanosleep`, `restart_syscall`, `clock_gettime`, `clock_nanosleep`, `epoll_wait`, `epoll_ctl`, `ppoll`, `epoll_pwait`, `timerfd_create`, `timerfd_settime`, `timerfd_gettime`, `epoll_create1`, `epoll_pwait2`), glibc runtime (`getrlimit`, `openat`, `newfstatat`, `set_robust_list`, `statx`, `rseq`, `clone3`), and allocator (`mremap`, `madvise`).

KVM ioctls, startup only unless noted: `KVM_GET_API_VERSION`, `KVM_CREATE_VM`, `KVM_CHECK_EXTENSION`, `KVM_GET_VCPU_MMAP_SIZE`, `KVM_GET_SUPPORTED_CPUID`, `KVM_CREATE_VCPU`, `KVM_SET_USER_MEMORY_REGION`, `KVM_SET_TSS_ADDR`, `KVM_CREATE_IRQCHIP`, `KVM_CREATE_PIT2`, `KVM_SET_REGS`, `KVM_GET_SREGS`, `KVM_SET_SREGS`, `KVM_SET_SIGNAL_MASK`, `KVM_SET_CPUID2`, `KVM_RUN` (both phases), and `KVM_IRQFD` (both phases) are measured in `soma-kvm`; `KVM_IOEVENTFD` (both phases) is reserved for virtio; `KVM_GET_REGS`, `KVM_GET_IRQCHIP`, `KVM_SET_IRQCHIP`, `KVM_SET_GSI_ROUTING`, `KVM_GET_CLOCK`, `KVM_SET_CLOCK`, `KVM_GET_MSRS`, `KVM_SET_MSRS`, `KVM_GET_FPU`, `KVM_SET_FPU`, `KVM_GET_LAPIC`, `KVM_SET_LAPIC`, `KVM_GET_MP_STATE`, `KVM_SET_MP_STATE`, `KVM_GET_VCPU_EVENTS`, `KVM_SET_VCPU_EVENTS`, `KVM_GET_PIT2`, `KVM_SET_PIT2`, `KVM_GET_XSAVE`, `KVM_SET_XSAVE`, `KVM_GET_XCRS`, `KVM_SET_XCRS`, `KVM_GET_NESTED_STATE`, and `KVM_SET_NESTED_STATE` are reserved for snapshot restore.
`FIONBIO` is the only non-KVM request and is admitted in both phases for the Rust runtime's `set_nonblocking`.
`TUNSETIFF` and every other request are killed.

## Not exercisable in this run

- The real `soma-vmm` binary: it does not exist yet, so the probe stands in and no `KVM_RUN` or stuck-vCPU containment was exercised.
- TAP transfer: no TAP endpoint was provided, so the manifest never carried a `tap` role; `TUNSETIFF` denial was proved on the KVM descriptor instead.
- `io.max`: the container has no block device the tests may throttle, so `io_max` stayed `None` and the `io` controller was only delegated.
- Snapshot ioctls: no code issues them yet, so their admission is proved only by the portable filter interpreter, not by the kernel.
- Multi-threaded `SECCOMP_FILTER_FLAG_TSYNC`: both installs happened while the installing process had one thread.
- PID reuse: the pidfd path was proved by `ESRCH` after exit, but the kernel was not driven to reuse the number.
- Crash at every step: only the cgroup `AlreadyExists` and the two descriptor-verification steps were driven to fail; the other pre-exec steps were not fault-injected.
- `RLIMIT_AS`: left unset because the musl allocator and thread stacks make a small bound fail before the tests run; the profile's memory bound is the cgroup.
- A glibc-linked VMM: every glibc-only entry stayed reserved because the probe is musl-linked.

## Observations

- `pivot_root` needs one descriptor above the inherited table, so the child applies `RLIMIT_NOFILE` right after the seal rather than before it; every other rlimit stays where the profile places it.
- The kernel reports a `SIGSYS` kill with `si_code CLD_DUMPED` even with `RLIMIT_CORE` 0 and dumpable 0, so `ExitReason::Signaled` carries `core_dumped: true` for seccomp kills and `is_seccomp_kill` ignores that flag.
- The report pipe reaches EOF a moment before the kernel commits post-exec credentials, so the launcher retries `/proc/<pid>/status` for up to 250 ms before judging the identity and capability state.
- `KVM_GET_API_VERSION` requires a zero argument; the probe originally passed garbage and received `EINVAL`, which was a probe bug rather than a filter effect.
