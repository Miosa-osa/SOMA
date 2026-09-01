# Splitting the durable machine host - 2026-09-01

Work in progress, committed unfinished. This records the design that is now in the tree, how
far it got, and what the next person would otherwise rediscover. **No live proof was taken.**
Nothing here is a measurement; every claim below is about code that compiles and passes the
workspace tests on a development host, not about a machine that ran inside a jail.

## The shape

[The prior investigation](2026-08-31-jailing-the-machine-host.md) established that the host
cannot be moved into the jail because `socket`, `bind`, `listen` and `accept4` are in
`soma_jail::NEVER_ALLOWED`. What is implemented here is the split that record identified.

- An unjailed **broker** keeps the listening socket, the Generation store, the head directory,
  the network lease and the Host Runtime registration. It is the existing `machine-host`
  process, unchanged in what it serves.
- A jailed **machine** keeps a sealed descriptor table and nothing else: an open `/dev/kvm`,
  the immutable root, the snapshot's memory image and state manifest, this Instance's already
  unlinked private head, and one pre-connected `SOCK_SEQPACKET`. It speaks only over that
  socket, using `recvfrom` and `sendto`, which both survive the seal.

`crates/soma-local/src/backend/kvm/held.rs` is the seam: `Held::Resident` is the old in-process
sandbox thread, `Held::Jailed` is the new one, and the lifecycle above asks both the same three
questions. A host is configured to jail by three environment variables
(`SOMA_JAIL_CGROUP_ROOT`, `SOMA_JAIL_ROOT_PARENT`, `SOMA_VMM_BINARY`); a host that names them
and cannot build a jail refuses to open the Backend rather than falling back, because falling
back would hold a live machine in an unjailed process.

## What crosses, and how

| Role | What it is | Opened by |
| --- | --- | --- |
| `Kvm` | `/dev/kvm`, read-write | broker |
| `RootDisk` | the Generation's immutable root artifact | broker |
| `Artifact(MemorySnapshot)` | `memory.raw` | broker |
| `Artifact(DeviceState)` | `state.somasnap` | broker |
| `OverlayHead` | the Instance's private head, reflinked then unlinked | broker |
| `Control` | one end of a `socketpair` | broker |

Three things could not travel and had to be given a way to.

- **The declared device set.** A machine must be built as the machine the Generation was
  certified as, and an artifact set can only agree with itself, so the set cannot be read back
  out of the snapshot. `soma_vmm::DeclaredDevices` now rides on `Generation` and on the launch
  packet.
- **The context identifier and the placeholder network.** The broker claims a network for an
  Instance and the jailed machine is built with one. Two derivations would let a machine be
  built under one identity and given a network leased to another, so `guest_cid_for` and
  `link_down_network` moved into `soma_vmm::sandbox::identity` and both sides call them.
- **A command's output.** One `SOCK_SEQPACKET` datagram cannot carry the sixteen mebibytes an
  Execute may produce, and a worker answering with as much as fitted would be reporting a
  different command than the one that ran. `Request::Output` reads bounded windows out of the
  receipt the worker already retains for exact replay.

## What the `Platform` stub cost, measured against the estimate

The prior document estimated "roughly 115 path operations across five crates". That count is an
estimate of grep surface, and the *restore* path is much narrower than it: `restore_sterile`
resolves exactly three paths (`state.somasnap`, `memory.raw`, and `/dev/kvm`), plus
`overlay.raw` when artifact verification is asked for. Converting them was
`SnapshotObjects` and `Hypervisor` in `soma-kvm`, about a hundred lines. The 115 figure covers
capture, installation, `soma-netd` and `soma-hostd`, none of which the jailed machine touches.

Two path dependencies were not in that count and would have failed silently inside an empty
root:

- `crates/soma-kvm/src/virtio/devices/rng/backend.rs` opened `/dev/urandom` for the virtio-rng
  device, on the restore path. It now uses the `getrandom` syscall, which the jail admits in
  both filter phases.
- `worker/commands.rs` opened `/dev/urandom` for each command's operation identity. Same fix.

The `Platform` trait's two-stage shape turned out to fit the restore exactly. `verify_and_restore`
is `Session::prepare` (restore a sterile machine and park it) and `authenticate_repair_and_ready`
is `Session::assign` (transfer this Instance's authority and drive to Ready). That mapping is
why a failure is reported as the half it happened in rather than as one indivisible launch.
`Machine::on_jailed_kvm` builds it from the manifest; a manifest naming a hypervisor and missing
a piece refuses service rather than serving the contract with nothing behind it.

**The self-referential borrow is the reason the sandbox thread exists and could not be removed.**
`RepairedHostControl<HostIo<'_>>` borrows the `SandboxMachine` so that committing repair can
retire the launch page, so a `Platform` implementation cannot store both in one struct. The
existing thread-and-channels `Session` already solves this, which is why it was moved into
`soma-vmm` rather than reimplemented. A future attempt to make `HostIo` own everything would
need `retire_launch_page` to work from an `Arc` handle rather than from `&SandboxMachine`.

## What is not done

- **Nothing ran.** No jailed machine was launched, no lifecycle was driven, and the contract
  benchmark was not re-measured against this branch. The only benchmark run recorded on this
  work was at commit `3e35d26` (the `soma-kvm` descriptor change alone, before any jail
  existed), and it reported 100 of 100 commands succeeded and 100 of 100 cleanups complete.
  That is a baseline, not a result for the split.
- **The broker must be privileged.** Building a user namespace with an identity map, a mount
  namespace with an empty root, and a cgroup v2 leaf needs more than an ordinary user has. The
  jail's own live tests run as root in a privileged container for the same reason. This is a
  real cost of the split and is not hidden: the machine loses everything, and the broker gains
  the privilege needed to take it away.
- **Egress and secrets are refused, not supported.** A jailed launch that needs a TAP or a
  secret is refused by name. The manifest has a `Tap` role for the first; the second needs a
  way for a secret to cross the boundary without touching the broker's filesystem.
- **A cold boot cannot be jailed.** The descriptor manifest has `Artifact(Kernel)` and
  `Artifact(Initramfs)` roles, but only the restore path is wired, so a Generation with no
  captured snapshot is refused rather than cold booted inside a jail.
- **The pool is bypassed when jailing.** A machine prepared inside the broker process is
  exactly the unjailed thing the split removes, so a jailing host claims nothing from it. A
  pool of jailed sterile workers is possible and not built.
- **`cpu.max` cannot exceed one CPU.** `soma_jail::CpuMax` validates `quota <= period`, so a
  jailed machine gets one CPU for its vCPU thread, its device loop and its runtime together.
  Whether that costs time to interactivity is exactly the kind of thing only a measurement can
  say, and none was taken.

## Where to pick it up

1. Build the static worker: `cargo build --locked -p soma-vmm --bin soma-vmm --target
   x86_64-unknown-linux-musl`. The jail root is empty, so a dynamically linked worker has no
   loader to start it.
2. As root, delegate a cgroup subtree (`cpu`, `memory`, `pids` in `cgroup.subtree_control`),
   then set `SOMA_JAIL_CGROUP_ROOT`, `SOMA_JAIL_ROOT_PARENT`, `SOMA_VMM_BINARY`, and
   optionally `SOMA_JAIL_LOG_DIR`, which is where the broker records each worker's PID and its
   attestation. A jailed machine has no name and no socket, so that line is the only way to
   find the process holding one Instance.
3. Drive five separate processes over one sandbox and compare `/proc/<pid>/ns/*` against PID 1.
