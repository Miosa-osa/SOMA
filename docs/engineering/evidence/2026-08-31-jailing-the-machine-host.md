# What it costs to put the durable machine host inside the jail - 2026-08-31

Measured on eval-1 at `d6c4230`, KVM backend, busybox at one vCPU, 1024 MiB of memory and
2048 MiB of storage, against the Generation store at `/srv/soma/dm-store`. Raw traces, syscall
verdicts, lifecycle envelopes, and benchmark records are under
[`raw/2026-08-31-jailing-the-machine-host/`](raw/2026-08-31-jailing-the-machine-host/).

This is a negative result. The durable machine host described in
[the durable machine host](2026-08-31-durable-machine-host.md) cannot be moved inside
`soma-jail` as it is shaped, and the reason is not the one that document predicted. No code
changed on this branch; what follows is measurement of the existing host against the existing
jail.

## The blocker that was expected, and the one that is actually there

The prior document closes by naming the obstacle:

> The KVM lifecycle reaches its Generation store, its head directory, and `/dev/kvm` by path, so
> moving it inside that jail means giving it directory descriptors and `openat` throughout.

That is true of the *paths*, and it is not the blocker. `openat` is admitted only in the jail's
startup phase and is dropped by the steady-state filter, so "`openat` throughout" is not
something the jail can be asked for without widening it. More decisively, the host's problem is
not what it opens. It is what it *is*: a process that binds a listening socket and accepts
connections on it.

`socket`, `bind`, `listen`, and `accept4` are not merely absent from the policy. They are in
`NEVER_ALLOWED`, the documented denial surface in
`crates/soma-jail/src/seccomp/denied.rs`, whose accompanying test proves the compiled BPF returns
`SECCOMP_RET_KILL_PROCESS` for each of them in *both* phases. A jailed process cannot be a
server. That is a deliberate property of this jail, not an oversight.

## What the host actually issues

The host was traced through a complete lifecycle with `strace -ff` following it from the
`machine launch` that spawns it to the `machine destroy` that ends it: launch, an exec that
writes a file, an exec that reads it back, an inspect, and a destroy, each a separate `soma`
process, all exiting zero. The retained traces are the host's main thread and the six threads it
cloned, in [`traces/`](raw/2026-08-31-jailing-the-machine-host/traces/).

Every observed syscall name was classified against the real policy tables - `soma_jail::NEVER_ALLOWED`
and `soma_jail::syscall_rules()` - by [`policy/classify.rs`](raw/2026-08-31-jailing-the-machine-host/policy/classify.rs),
so the verdicts below are the jail's own tables rather than a reading of them.

The host's main thread issues its first `accept4` at line 207 of 249. Everything from there on is
the serving loop, and it is the part that would have to survive the steady-state filter
([`serving-phase-verdicts.txt`](raw/2026-08-31-jailing-the-machine-host/policy/serving-phase-verdicts.txt)):

| Syscall | Verdict |
| --- | --- |
| `accept4` | killed always; documented denial surface |
| `unlink` | killed always; documented denial surface |
| `setsockopt` | killed always; absent from the policy table |
| `fcntl` | startup-only, so killed in steady state |
| `close`, `exit_group`, `futex`, `munmap`, `recvfrom`, `sendto`, `sigaltstack` | admitted |

Four of the eleven syscalls in the serving loop do not survive. Over the whole host lifetime
([`whole-lifetime-verdicts.txt`](raw/2026-08-31-jailing-the-machine-host/policy/whole-lifetime-verdicts.txt))
fifteen distinct syscalls are killed always - `socket`, `bind`, `listen`, `accept4`, `access`,
`chmod`, `dup2`, `execve`, `mkdir`, `mkdirat`, `openat2`, `readlink`, `sched_getaffinity`,
`setsockopt`, `unlink` - and eleven more are admitted only before the seal.

One of those fifteen is not a real obstacle and is named here so the count is not read as worse
than it is: `execve` is the host binary being started by its parent, and inside a jail that is
the launcher's `execveat`, which the startup filter admits. The other fourteen are the host's own
work.

Three of these are worth naming individually, because they are not incidental:

- `readlink`. The overlay head is created and immediately unlinked, and the host recovers its
  path by reading `/proc/self/fd/<n>` (`crates/soma-local/src/backend/kvm/boot.rs:221`). The jail
  refuses service unless procfs is invisible, and `readlink` is in the denial surface. The head
  machinery depends on both.
- `unlink`. The host removes its own socket on the way out and clears sockets nothing answers on.
  That is how an Instance stops being reported as served.
- `dup2`. The host rewires its own standard descriptors. The jail launcher seals the descriptor
  table itself, so this is work the jail has already taken over.

## The machine half is ready; the hosting half is not

The same traces record every `ioctl` the machine issues. Twenty-seven distinct KVM requests were
observed, from `KVM_CREATE_VM` and `KVM_SET_USER_MEMORY_REGION` through 351 `KVM_RUN` calls
([`observed-ioctls.txt`](raw/2026-08-31-jailing-the-machine-host/policy/observed-ioctls.txt)).
**Every one of them is already on the jail's ioctl allowlist.** The gap is two entries, and
neither is KVM:

| Not allowed | What issues it |
| --- | --- |
| `BTRFS_IOC_CLONE` | reflink-cloning the overlay head from its template |
| `FS_IOC_FIEMAP` | mapping the template's extents |

Both belong to head creation, which is filesystem work on a path. So the jail's syscall and ioctl
policy was designed correctly for the machine. What it refuses is the hosting: the socket, the
directory, the head, and the standard-descriptor rewiring.

## What this means

The machine can be jailed. The host cannot, because everything that makes it a host is the set of
operations the jail's denial surface exists to forbid. Splitting along that line is not a
workaround; it is the architecture `soma-vmm` already declares. Its descriptor manifest names
exactly the roles this would need - `Kvm`, `OverlayHead`, `Artifact(Kernel)`,
`Artifact(MemorySnapshot)`, `Control` - and its `Control` role is a pre-connected `SOCK_SEQPACKET`
socket precisely so that the jailed process never binds or accepts.

The shape that fits, then, is a split rather than a move:

- An unjailed broker keeps the socket, the accept loop, the state root, and the head directory.
  It resolves the Generation store, opens `/dev/kvm`, creates and opens the head, opens the
  kernel and the snapshot, and hands all of it over as a sealed descriptor table.
- The jailed `soma-vmm` holds the machine and speaks only over its pre-connected control
  descriptor, using `recvfrom` and `sendto`, which both survive the seal.

A caller could not tell the difference, which was the requirement. But the host process as it
exists today does not move inside the jail; it becomes the thing outside it.

### What is not proved here

- Nothing was built. This is a measurement of the existing host against the existing jail, and no
  jailed machine was run.
- The remaining work is not small. `soma-vmm`'s `Platform` is `UnavailablePlatform`, a stub that
  restores nothing, so the machine lifecycle would have to reach it from `soma-local`. The path
  operations that would have to become descriptor operations number roughly 115 across
  `soma-kvm`, `soma-generation`, `soma-storage`, `soma-netd`, and `soma-hostd`. That was counted
  by grep, not by attempting it, and it is an estimate of surface rather than of difficulty.
- One trace, one host, one lifecycle. A different guest or a different Generation could issue
  syscalls this one did not.

## The unjailed host still does what it did

Because no code changed, this is a re-measurement rather than a regression check, and it is
recorded so the negative above is not mistaken for something having broken.

Five separate `soma` processes over one sandbox, envelopes in
[`lifecycle/`](raw/2026-08-31-jailing-the-machine-host/lifecycle/):

| Process | Command | Exit | Result |
| --- | --- | ---: | --- |
| 1 | `machine launch` | 0 | `state: ready` |
| 2 | `machine exec` writing `/tmp/proof.txt` | 0 | `exited: 0`, file listed at 31 bytes |
| 3 | `machine exec` reading it back | 0 | `persisted-by-the-first-process\n` |
| 4 | `machine inspect` | 0 | `state: ready` |
| 5 | `machine destroy` | 0 | `state: destroyed` |

The contract benchmark, three independent hundred-way runs at this build, against a manifest
recording `git_revision d6c4230` and `worktree_clean: true`:

| Run | Attempted | Command succeeded | Cleanup complete | TTI p50 | TTI p95 | Wall |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 100 | **100** | **100** | 1562.80 ms | 3753.18 ms | 6.45 s |
| 2 | 100 | **100** | **100** | 1625.80 ms | 3614.53 ms | 5.85 s |
| 3 | 100 | **100** | **100** | 1302.49 ms | 4258.23 ms | 6.54 s |

After each run and after the five-process lifecycle, the head directory held no entries, no
socket survived under the state root, and no `machine-host` process remained.
