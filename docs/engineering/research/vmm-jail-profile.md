# SOMA VMM jail profile v1

## Decision

One `soma-vmm` process runs as one ephemeral unprivileged UID and GID inside dedicated user, mount, PID, network, IPC, and UTS namespaces and one cgroup v2 leaf.
A narrow privileged broker prepares resources and transfers only already-open KVM, TAP, disk, artifact, event, control, and log descriptors.
The VMM never retains `CAP_NET_ADMIN`, `CAP_SYS_ADMIN`, a host root path, registry access, or control-plane credentials.

## Construction order

The launcher records ownership, creates the cgroup and namespaces, opens verified resources, forks or clones the child, sets `PR_SET_PDEATHSIG`, applies rlimits, closes every unapproved descriptor, changes to an empty root, installs `no_new_privs`, installs the phase-specific seccomp filter, and execs the content-addressed VMM.
The child verifies its descriptor manifest before touching KVM.
The parent uses pidfd identity and never trusts a reused numeric PID.

The mount namespace contains an empty read-only root, no procfs after startup, no sysfs, no device tree, and only descriptor-backed anonymous or sealed artifacts.
The network namespace has no interface except the transferred TAP endpoint required by the guest path.
The cgroup fixes memory, swap, CPU, pids, and I/O bounds and uses `memory.oom.group=1`.

## Seccomp

Filters are generated from traced and reviewed cold-restore, steady-state, failure, timeout, and cleanup paths.
Startup admits only the exact KVM, eventfd, epoll, timerfd, pidfd, memory, file, signal, threading, and descriptor operations required by the implementation.
Steady state removes setup-only ioctls and filesystem mutation.
Unknown syscalls and ioctl commands kill the process and produce incomplete-cleanup evidence for reconciliation.

The VMM has no `execve` after startup, socket creation, DNS, ptrace, mount, namespace creation, module loading, keyring, BPF, perf, fanotify, inotify, userfaultfd, or arbitrary ioctl authority in version 1.
Diagnostics use bounded inherited descriptors and never relax the production filter.

## Failure and evidence

Parent death, OOM, signal, panic, seccomp kill, stuck `KVM_RUN`, or broker loss initiates pidfd-based termination and idempotent ledger cleanup.
The launcher does not reuse a jail or UID until reconciliation proves all resources gone.

Modules are `jail/spec`, `jail/descriptors`, `jail/namespaces`, `jail/cgroup`, `jail/seccomp`, `jail/process`, and `jail/reconcile`.
Acceptance requires adversarial descriptor injection, namespace escape, syscall and ioctl denial, resource exhaustion, parent death, PID reuse, crash-at-every-step, stuck-vCPU containment, and zero residual resource tests on Ubuntu 24.04.
