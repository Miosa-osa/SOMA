# How one SOMA sandbox works today

This guide follows one sandbox from a Template document to cleanup on the Linux KVM path as it exists on `main`, and says where the chain is not yet joined.
It is written for an engineer who has never opened the repository.
It describes only what the code does and what the retained evidence shows.
Every number is a single-sample, debug-build observation from one named evidence file unless the sentence says otherwise, and none of them is a benchmark.

Status words in this guide are the five terms defined in [the engineering standard](../standards/sota-engineering-standard.md#status-vocabulary): designed, component-tested, live-proved, integrated, production-admitted.
A live-proved sentence names the commit the run was made on and links its evidence, and a run whose code has since changed is called historical.
[The claim ledger](../claim-ledger.md) carries the same statuses in one table.

The vocabulary is the one in [CONTEXT.md](../../CONTEXT.md): Template, Generation, Snapshot, Machine, Instance, Launch, Ready, Repair, Backend, Receipt, Machine shape, vCPU, Host, Workload runtime, and Guest agent.
Template Lock is defined in [the template system](../architecture/template-system.md) and [ADR 0022](../adr/0022-compose-templates-into-generation-locks.md), not in CONTEXT.md.

This guide adds machine terms that neither CONTEXT.md nor [GLOSSARY.md](../../GLOSSARY.md) carries:

- The launch page is one 4 KiB guest-physical page that carries fresh per-Instance material; the owner process writes it once and then retires it.
- vsock is the host-to-guest socket transport, and virtio-mmio is the memory-mapped virtio transport that gives each device one 4 KiB register page.
- An irqfd is a file descriptor that raises one guest interrupt when it is written, an ioeventfd is a file descriptor KVM signals instead of exiting when the guest writes one address, and a GSI is the global system interrupt number the in-kernel controller routes.
- The PVH entry point is the paravirtualized boot entry an x86_64 Linux ELF image advertises in a Xen ELF note.
- EROFS is the read-only Linux filesystem format of the immutable root, an initramfs is the archive the kernel unpacks into memory before the root switch, and OverlayFS is the union filesystem that stacks a writable head over a read-only lower layer.

## 1. The one-paragraph answer

A SOMA sandbox is one hardware-isolated Linux virtual machine, the Machine, created and owned by one host-side process for exactly one Instance lifetime.
At build time the Template compiler turns a Template into a Template Lock.
The Generation compiler then turns a lock and the normalized image tree into an immutable Generation.
At Launch time the owner process creates the Machine through Linux KVM, boots the Generation, delivers fresh per-Instance material through a dedicated launch page, waits for the Guest agent inside the Machine to complete Repair and authenticate over vsock, and only then reports Ready.
Commands are argument vectors executed by the Guest agent over the authenticated channel, and shutdown is an authenticated request followed by cleanup evidence.
That path is live-proved twice on a real Ubuntu 24.04 x86_64 host, both times by ignored tests in `crates/soma-kvm`: at `71161ea` a cold boot of a compiled `busybox` Generation and a compiled `node:22` Generation returned one bounded command from each and proved cleanup, recorded in [the first sandbox command evidence](../evidence/2026-08-29-x86_64-first-sandbox-command.md), and at `7c1127d` one captured `node:22` Generation was restored thirteen times into independent authenticated Instances, recorded in [the x86_64 snapshot restore evidence](../evidence/2026-08-29-x86_64-snapshot-restore.md).
Both runs are historical: the code has since moved to initramfs layout v3, launch-page schema 3, and an authenticated readiness receipt after restore, so on current bytes the path is component-tested until it is run again.
The owner process in both proofs is the test process, not the designed `soma-vmm` binary, and prepared workers, a jail around the real VMM, network attach to the Machine, and any KVM lifecycle behind the CLI or MCP server remain designed.
Section 4 lists those gaps in the repository's own words.

## 2. Build time: Template to Template Lock to Generation

Build time is the slow, occasional work that must never sit on the Launch path.
Two crates own it: `crates/soma-template` produces a Template Lock, and `crates/soma-generation` produces a Generation.

### 2.1 Template to Template Lock

A Template is a `soma.template/v1alpha1` TOML document; [Creating a Template](creating-templates.md) shows how to write one.
`crates/soma-template/src/schema/parse.rs` bounds the document to 256 KiB, reads it through a claim-tracking reader, and rejects any unknown key with its full dotted path.
`crates/soma-template/src/compose` builds a flat ordered module list with transitive requirements from an in-memory registry.
The registry in `crates/soma-template/src/module/builtin.rs` holds exactly four data-defined modules: `agent/claude-code@1`, `agent/osa@1`, `tools/git@1`, and `tools/shell@1`.
`crates/soma-template/src/validate` applies the rejection classes from the template design against a policy ceiling, Backend capabilities, an OCI resolver, and a filesystem oracle.
`crates/soma-template/src/resolve.rs` pins the mutable image reference to an exact OCI manifest digest and platform through the `OciResolver` seam.
`crates/soma-template/src/lock` encodes the result as the canonical `SOMALOCK` version 1 byte layout, and `crates/soma-template/src/identity.rs` defines the `LockId` as the SHA-256 of those bytes.

The lock binds the resolved digest and platform, ordered module identities and digests, the effective command, resources, the normalized network envelope, lifecycle, the environment contract, secret references, the policy ceiling, and the Backend capabilities.
It excludes the Template name, description, the mutable image text, TOML layout, and every secret value.
The retained golden lock for the specification example in `crates/soma-template/tests/fixtures/example-lock.hex` has the `LockId` recorded in `crates/soma-template/tests/fixtures/example-lock.id`:

```text
sha256:cb0a718c20a2bd31dc24490d4f06ec45a9dfbd56734cc7a3a9ab8ac3766c8194
```

`crates/soma-template/src/revision.rs` projects a decoded lock onto the compiler's input as a `TemplateRevision` view.
Two boundaries matter for the story below.
The `OciResolver` and the filesystem oracle have deterministic test implementations only, so nothing on `main` contacts a registry or inspects a real root filesystem during resolution.
No Generation has yet been built from a Template Lock: the live KVM test in section 3 constructs the compiler's `TemplateRevision` directly from an imported image, and the specification example's allowlist envelope makes `TemplateRevision::shape()` fail closed with `UnrepresentableNetwork` before its two vCPUs are considered.
`crates/soma-generation/tests/template_boundary.rs` records the one-vCPU boundary over a fully denied document.
The compiler's own contract is in `crates/soma-generation/src/generation/template.rs`: platform `linux/amd64`, one vCPU, memory from 128 MiB through 3 GiB, writable storage of at least 64 MiB in 4 MiB units, and a lifetime of at most thirty days.

### 2.2 Template Lock to Generation

`crates/soma-generation` has three stages before the compiler runs.
`import_oci_layout` verifies an extracted OCI layout, selects one platform manifest, and stores every layer by digest.
`normalize_oci_rootfs` applies the selected layers into one canonical logical tree without extracting guest paths onto the Host, streams file contents into the content store, and publishes a tree manifest.
`compile_generation` in `crates/soma-generation/src/generation/compile.rs` then takes one `TemplateRevision`, that `NormalizedRootfs`, a content store, `CompilerProfile::v1()`, and a `BuildHost` naming the staging directory, the pinned toolchain, and four Machine inputs: the kernel, its configuration text, the early-init executable, and the Guest agent executable.
There is no secret input: `MachineInputs` lost its fifth field when [ADR 0024, per-Instance guest responder authority](../adr/0024-per-instance-guest-responder-authority.md) moved the responder secret to the launch page.

The compiler design in [the Generation compiler research](../research/generation-compiler.md) has six phases.
Phases 1 through 3 and 6 are component-tested: resolve and verify inputs, emit the canonical filesystem stream, build and independently verify every artifact, and publish atomically with the manifest last.
Phase 4, live boot and Snapshot capture, remains Linux KVM work outside the portable compiler.
`CompiledCandidate.unimplemented` therefore names `BootAndCapture`, and its manifest carries `SnapshotBinding::Absent` until a capture is installed.
Phase 5 is implemented: `install_snapshot` publishes the exact captured objects, `certify_candidate` verifies them against the Candidate and their internal state, and `promote_candidate` publishes the ready manifest last.
A compiled Generation therefore has no Snapshot of its own and is cold-booted; capture and restore are driven separately by `crates/soma-kvm/src/x86_64/snapshot/`, as section 4 records.

The artifacts a compiled Generation contains, and where each comes from:

| Artifact | Producer | What it is |
| --- | --- | --- |
| EROFS root | `generation/erofs.rs` streams the canonical tree as an ordered tar into the pinned EROFS formatter 1.9.4 in `--tar=f` mode through standard input; `generation/erofs_reader.rs` walks the image back and requires exact equality with the tree | The immutable read-only lower filesystem, shared by every Instance of the Generation |
| Overlay template | `generation/overlay.rs` runs the pinned ext4 formatter under a private configuration and fake time, creates the empty `upper` and `work` directories with `debugfs`, and checks with `e2fsck -fn` | One sterile ext4 image per certified writable size class; each Instance gets a private copy of it as its writable head |
| Kernel | `generation/kernel.rs` verifies the ELF headers, the `XEN_ELFNOTE_PHYS32_ENTRY` note, and the loaded addresses; `generation/kernel_config.rs` verifies the required built-in facilities | The pinned uncompressed x86_64 Linux ELF image booted through the PVH entry |
| Initramfs | `generation/initramfs.rs` writes a deterministic `newc` archive, layout version 3, with fixed modes, zero timestamps, and an allowlisted entry set | Carries `/init` and `/bin/soma-guest-agent`, which are both the Guest agent, and the `/dev/console` and `/dev/null` nodes PID 1 needs; it carries no secret, because [ADR 0024, per-Instance guest responder authority](../adr/0024-per-instance-guest-responder-authority.md) removed layout v2's `/etc/soma/responder.key` |
| `SOMAGEN` manifest | `generation/manifest` encodes fixed-order binary groups: source OCI identity, tree identity, root, overlay, kernel, initramfs, Guest agent, the complete kernel command line, machine, device, and CPU contracts, Machine shape, the absent Snapshot, Repair policy, and the Template fields | The canonical description of every artifact and contract |
| `GenerationId` | `generation/identity.rs` | `sha256:` plus the SHA-256 of the manifest bytes and nothing else |

The kernel command line is composed once, in `generation/contracts.rs`, and the Machine in section 3 must produce the same bytes.
The Host build binds no builder-image digest, and the CPU template digest covers a declaration statement rather than defined CPUID masks; both are recorded in the compiler research status.

The pinned kernel is its own build proof.
[The kernel build evidence](../evidence/2026-08-29-x86_64-pvh-kernel-build.md) records Linux `v6.12.107` built with no network access inside a digest-pinned Ubuntu 24.04 image, with no loadable modules, no PCI, no ACPI, the five virtio-mmio drivers, EROFS, ext4, OverlayFS, and `CONFIG_DEVMEM=y` so the Guest agent can map the launch page.
Two consecutive builds on the same 24-core host produced the byte-identical `vmlinux-6.12.107-soma-v1`, 21,530,432 bytes, SHA-256 `f1af3a142fa39916cfac425a01b16b5f328279823533421c9eec3f192c05b746`, with `make vmlinux` wall times of 49.9 s and 52.6 s; cross-host reproducibility is untested.

Real artifacts from the retained `busybox:stable-musl` run in [the first sandbox command evidence](../evidence/2026-08-29-x86_64-first-sandbox-command.md), all on the Ubuntu 24.04 x86_64 host it names.
These are historical observations from a revision that still used initramfs layout v2, so the initramfs digest and every `GenerationId` below are no longer reproducible:

| Artifact | Digest | Size |
| --- | --- | ---: |
| Source image index (`docker save` on Docker 29.3.0, `linux/amd64` selected) | `sha256:3c6ae8008e2c2eedd141725c30b20d9c36b026eb796688f88205845ef17aa213` | not recorded |
| Normalized tree | `sha256:5c47256d83adfa1d6162df9991dcd5e0f65660111e7e3f9391472069356094e1` | 424 entries |
| EROFS root, erofs-utils 1.9.4 | `sha256:6eeb5664f2ec671974c623638d4d4047cbfe5f6d5d03c41e3ed8f7d0f430ea5e` | 1,511,424 bytes |
| Overlay template, 64 MiB class, e2fsprogs 1.47.0 | `sha256:ada7d6262e53685413ca7fc2159f61a8f8339e78ab34eb7280bf329f4f48a3cb` | 67,108,864 bytes |
| Kernel | `sha256:f1af3a142fa39916cfac425a01b16b5f328279823533421c9eec3f192c05b746` | 21,530,432 bytes |
| Guest agent, static musl, stripped | `sha256:6f3f657366a422d497b66f76b75cead972c6ad2cc9d1c00cdd38057ce0ca0eb0` | 819,384 bytes |
| Initramfs, layout v2 | `sha256:e1605383813d4e34f9601699831802e683801caa720892412c8aaabfa5836ad1` | 1,640,960 bytes |
| `GenerationId` | `sha256:0b6a3a4ea53accf5d4aabfd3156f29653297de23497ac0d58b51ad506ddbebcc` | manifest digest |

The same evidence shows why the identity moved on that revision.
The immediately preceding run produced `GenerationId` `sha256:4a064668c586d94aa83643e86304f91278cb2e27549e47986d68dcfb9a72a73a` from the same tree, root, and overlay digests, because the responder key bound into the initramfs was generated per run.
The EROFS root digest was identical in every run.
Under layout v3 that source of movement is gone: the archive carries no secret, so one tree, root, overlay, kernel, and agent now produce one stable `GenerationId`.

A reader who reproduces the test will not see the Guest agent digest or either `GenerationId` above.
The retained runs used the pre-split Guest agent.
The same evidence records that `boot.rs` was later split into `boot/devices.rs` without changing behavior, and that the agent rebuilt from the committed source is 823,480 bytes with SHA-256 `d4c29837dd72c3fb8ec533e7c148a61aed1d890930dbe73eec558882a0e6b132`.
That agent produces `GenerationId` `sha256:537c06203beb409333c11be41c379d7812237e16a95b3ad5652012dc14a3f795` and reached Ready 137.6 ms after `KVM_RUN`, outside the 164 ms to 193 ms range in section 3.8, which was not re-recorded against it.

The `node:22` Generation from the same evidence is the realistic size.
Its normalized tree is `sha256:2e48535fca4ce401af5271cd6df8e39fa6a723fe4ee0abd570dfcf02f2a1e41e` with 33,512 entries, its EROFS root is `sha256:48a6cf92bd0b4a57ee7ea87f0d3efe774ad26bd47d6db4ed6c23c83dcfe8aa48` at 1,129,172,992 bytes, its 1 GiB overlay template is `sha256:0a24f757ffd3e6208621593cdda61b0e9bb6c2bb6d5b59d5bf943a4379105cc2` at 1,073,741,824 bytes, and its `GenerationId` is `sha256:190ad2f23b4b9f0dd899e8b88d8c51dd7927f44a569fdad68bf53ab7170a978d`.
That whole test took 369 s of wall time, almost all of it in import, normalization, EROFS formatting and verification of the 1.1 GB tree, and the 1 GiB template build; that is build time, not Launch time.
The development Mac's earlier `node:22` normalization digest was not reproduced because the two hosts held different `node:22` image revisions, so a same-input cross-host comparison is still open.

## 3. Launch time: the proven cold-boot path

Everything in this section is what `crates/soma-kvm/tests/x86_64_sandbox_boot.rs` does and what [the first sandbox command evidence](../evidence/2026-08-29-x86_64-first-sandbox-command.md) recorded when it ran.
The test drives `SandboxMachine` in `crates/soma-kvm/src/x86_64/sandbox.rs` directly; the authenticated protocol glue over the machine's byte channel lives in the test's `session.rs` and `control.rs` because `soma-guest` is a private crate that the public `soma-kvm` package cannot depend on.

The two earlier machine proofs still pass and sit underneath this one.
[The halt-guest evidence](../evidence/2026-08-29-x86_64-kvm-halt-guest.md) proves the KVM floor: one VM, one 128 MiB private memory slot, one protected-mode vCPU, port I/O exits, `KVM_EXIT_HLT`, and balanced descriptors, with a total of 38,401,598 ns for the debug-build single sample.
[The PVH kernel-boot evidence](../evidence/2026-08-29-x86_64-pvh-kernel-boot.md) proves the machine contract: the pinned kernel enters through the PVH note, prints a challenge-bound sentinel from a static `/init`, and exits through the `reboot=k` reset pulse, with the `Run` phase between 115 ms and 586 ms across eight debug-build runs on a busy host.

### 3.1 Creating the Machine

`SandboxMachine::create` builds every owned resource in contract order without running anything.
The order is the one in the code:

1. Read the kernel and the initramfs from the Generation store under fixed byte limits.
2. Open `/dev/kvm`, run the capability probe, create the VM with `KVM_CREATE_VM`, and map guest RAM as one anonymous `MAP_PRIVATE | MAP_NORESERVE` region registered as memory slot 0 at guest-physical 0.
   RAM is a multiple of 4 KiB between 128 MiB and 3 GiB.
3. Set the TSS window at `0xfffbd000`, create the in-kernel interrupt controller with `KVM_CREATE_IRQCHIP`, and create the PIT with `KVM_CREATE_PIT2`.
   The PIT is required: without it the guest's local APIC timer is never calibrated and a `nanosleep` in `/init` never returns.
4. Bind the five device models to the fixed MMIO bus in `crates/soma-kvm/src/x86_64/devices.rs`.
5. Map and register the launch page as memory slot 1.
6. Compose the kernel command line in `crates/soma-kvm/src/x86_64/cmdline.rs` and load the kernel segments, the initramfs, and the PVH boot pages into RAM.
7. Create vCPU 0, install the CPUID template with `KVM_SET_CPUID2`, and install the contract's 32-bit protected-mode registers with `RBX` pointing at the PVH start page.
8. Register the serial irqfd on GSI 4, one edge-triggered irqfd per device slot on GSIs 5 through 9, and one ioeventfd per queue-notify address with `datamatch` equal to the queue index.

The five virtio-mmio devices are at fixed addresses and interrupts, and the guest learns about them only from the command line:

| Slot | MMIO page | GSI | Device | Host-side backing in the proven path |
| ---: | --- | ---: | --- | --- |
| 0 | `0xd0000000` | 5 | Immutable root block, read-only | The EROFS root artifact, opened read-only |
| 1 | `0xd0001000` | 6 | Private overlay block | A private copy of the sterile ext4 template, opened read-write |
| 2 | `0xd0002000` | 7 | Network | The link-down loopback placeholder; no TAP exists |
| 3 | `0xd0003000` | 8 | Vsock control | The `HostEndpoint` byte stream on the fixed control port |
| 4 | `0xd0004000` | 9 | Entropy | A fresh `/dev/urandom` handle |

The launch page is a second KVM memory slot at guest-physical `0xd0100000`, above RAM and the MMIO window, so it can never be part of a Snapshot.
The command line the Machine composed, which the test requires to equal the `command_line` bytes bound into the `SOMAGEN` manifest:

```text
console=ttyS0 reboot=k panic=1 nomodule random.trust_cpu=off pci=off acpi=off noapic cryptomgr.notests virtio_mmio.device=4K@0xd0000000:5:0 virtio_mmio.device=4K@0xd0001000:6:1 virtio_mmio.device=4K@0xd0002000:7:2 virtio_mmio.device=4K@0xd0003000:8:3 virtio_mmio.device=4K@0xd0004000:9:4 rdinit=/init soma.lower=/dev/vda soma.upper=/dev/vdb
```

### 3.2 Delivering launch material and starting

Before the vCPU runs, the test generates fresh launch material with `soma_guest::HostLaunchMaterial::generate`.
The page binds the `GenerationId`, a fresh 16-byte Instance identity, a 16-byte operation identity, a launch nonce, a Noise pre-shared key, an entropy seed, and the non-secret network identity: vsock CID 3, network generation 1, MAC `02:53:4f:4d:41:01`, IPv4 `10.0.0.2/24`, gateway and resolver `10.0.0.1`, and a wall-clock sample.
`write_launch_page` copies the 4,096 bytes into slot 1 exactly once; the material type cannot be delivered twice.
`start` spawns the device thread, which services the queues through epoll with a bounded budget per wakeup, and starts vCPU 0 on its own thread inside `KVM_RUN`.
The Machine shape in the retained run was 1 vCPU, 256 MiB RAM, and the 64 MiB writable class.

### 3.3 Kernel boot to `/init`

The kernel enters at the PVH address, sees the three-entry memory map, selects `kvm-clock`, uses the PIC in virtual wire mode because of `noapic`, and registers the five devices.
The retained serial log shows the discovery and the two block devices:

```text
[    0.104289] virtio-mmio: Registering device virtio-mmio.0 at 0xd0000000-0xd0000fff, IRQ 5.
[    0.105035] virtio-mmio: Registering device virtio-mmio.1 at 0xd0001000-0xd0001fff, IRQ 6.
[    0.107277] virtio-mmio: Registering device virtio-mmio.2 at 0xd0002000-0xd0002fff, IRQ 7.
[    0.107973] virtio-mmio: Registering device virtio-mmio.3 at 0xd0003000-0xd0003fff, IRQ 8.
[    0.109234] virtio-mmio: Registering device virtio-mmio.4 at 0xd0004000-0xd0004fff, IRQ 9.
[    0.120852] serial8250: ttyS0 at I/O 0x3f8 (irq = 4, base_baud = 115200) is a 16550A
[    0.121846] random: crng init done
[    0.122930] virtio_blk virtio0: [vda] 369 4096-byte logical blocks (1.51 MB/1.44 MiB)
[    0.124908] virtio_blk virtio1: [vdb] 16384 4096-byte logical blocks (67.1 MB/64.0 MiB)
[    0.135096] Run /init as init process
```

`random: crng init done` came from the virtio entropy device before the Guest agent's own reseed.
The full 168-line log is retained by the test under `target/tmp/x86_64-sandbox-boot/busybox/serial.log`.

### 3.4 Early init inside the guest

`/init` is the Guest agent, `crates/soma-guest-agent/src/main.rs`, and it stays PID 1 for the life of the Machine.
`crates/soma-guest-agent/src/boot.rs` performs the early-init sequence itself, with every step typed and a 10 s budget:

1. Mount devtmpfs on `/dev`, procfs on `/proc`, and sysfs on `/sys`.
2. Wait for exactly the two virtio block devices, `/dev/vda` and `/dev/vdb`.
3. Verify the EROFS superblock of `/dev/vda` and mount it read-only at `/mnt/lower`.
4. Verify the ext4 superblock of `/dev/vdb` and mount it read-write at `/mnt/upper`.
5. Verify that the head contains only `lost+found`, `upper`, and `work`, and that `upper` and `work` are empty directories, creating them when absent; anything else is treated as tenant state or tampering and fails.
6. Mount OverlayFS at `/mnt/root` with the EROFS lower and the private `upper` and `work` directories.
7. Move `/dev`, `/proc`, and `/sys` into the composed root.
8. Move the composed root over `/` and enter it with `chroot`, exactly as `switch_root` does, because `pivot_root` cannot leave the initial ramfs.

The agent takes no secret from the initramfs, because layout v3 carries none.

The serial log shows the result:

```text
[    0.135910] erofs: (device vda): mounted with root inode @ nid 36.
[    0.173387] EXT4-fs (vdb): mounted filesystem be4ea2dd-333d-4fa2-a314-aa8832de9f2d r/w with ordered data mode. Quota mode: disabled.
[    0.174294] overlayfs: "xino" feature enabled using 2 upper inode bits.
```

Two contract bugs were found and fixed by the first boots: the sterile template already carried `upper` and `work` so the agent had to create or verify rather than create, and the tmpfs option string must not repeat `nosuid,nodev` because the new mount API rejects unknown parameters.

### 3.5 Repair, from launch page to Ready

The agent's lifecycle is a typestated controller in `crates/soma-guest-agent/src/repair.rs` with exactly these phases: `Captured`, `MaterialAccepted`, `EntropyRepaired`, `TransportFresh`, `IdentityRepaired`, `NetworkRepaired`, `Authenticated`, `Probed`, `Ready`, `Running`, `Stopping`, and terminal `Poisoned`.
An out-of-order or duplicate transition is unrepresentable through the safe API, and a runtime ledger re-checks the same order.
Any failure poisons the controller and powers the Machine off.

Launch page consumption.
`crates/soma-guest-agent/src/launch_page.rs` maps guest-physical `0xd0100000` through `/dev/mem`, polls every 2 ms until the `SOMA-LAUNCH-PAGE` domain appears, copies the page once into locked zeroizing memory, overwrites the mapping with zeroes through volatile stores, re-reads every byte to verify the erase, and only then parses the locked copy.
The host side observes the domain vanish by polling the slot every 1 ms, which is the `LaunchPageConsumed` milestone and lags the guest by up to 1 ms.

Entropy Repair.
`crates/soma-guest-agent/src/entropy.rs` reads 64 bytes from `/dev/hwrng`, which the virtio entropy device serves, combines them with the 64-byte launch seed, credits the 128 bytes to the kernel with `RNDADDENTROPY`, forces a reseed with `RNDRESEEDCRNG`, and proves `getrandom` no longer blocks.

Transport.
`crates/soma-guest-agent/src/control.rs` connects from the assigned CID to the Host CID on the fixed control port `0x534f4d41`.
The vsock device model accepts one stream connection at a time on that port and answers any other port with `RST`.

Identity Repair.
`crates/soma-guest-agent/src/identity.rs` writes the hostname `soma-` plus the first six Instance bytes in hex, writes `/etc/machine-id` from the Instance identity through an atomic rename, mounts fresh tmpfs on `/run` and `/tmp`, and sets the wall clock from the launch-page time sample.

Network Repair.
`crates/soma-guest-agent/src/network_repair.rs` forces `eth0` down, installs the MAC, address, and netmask through the classic `ioctl` interface, raises `lo` and `eth0`, adds the default route, and writes `/etc/resolv.conf` and `/etc/hosts`.
Zero transport violations were recorded even though the agent installs the MAC, because the Machine reports the launch-page MAC in virtio-net configuration space and Linux skips the driver call when the address already matches.
No frame left the Machine; the network device sits behind the link-down loopback backend.

Authentication.
The host side calls `soma_guest::HostControl::connect` over the Machine's `ControlChannel`, and the guest side calls `GuestControl::connect` with the responder secret it took from the launch page.
The protocol is `Noise_NKpsk0_25519_ChaChaPoly_BLAKE2s`, pinned in `crates/soma-guest/src/lib.rs`, with the Instance pre-shared key from the launch page.
Neither peer accepts a caller-supplied responder key: the Host samples the keypair fresh for this Instance, delivers the private half at launch-page byte 247, and keeps the public half itself, so no responder key is bound into the Generation manifest at all.

Repair commit.
The host side sends `Prepare`, which carries nothing but the Launch operation, the guest reports `RepairComplete`, and the host side commits Repair by verifying that all 4,096 bytes of the launch page read as zero and removing slot 1 with a zero-length `KVM_SET_USER_MEMORY_REGION`; that is the `LaunchPageRetired` milestone.
The agent then prints `soma-guest-agent: ready` and enters the request loop.
Ready means Repair is complete and reported under this Instance's own authenticated session; [ADR 0039](../adr/0039-repair-report-alone-proves-readiness.md) records why a command round trip is not part of it.

### 3.6 Execute

An Execute request is one absolute program path plus an argument vector, a timeout, and an output allowance, with no shell.
`crates/soma-guest-agent/src/executor.rs` runs it as root with a fixed environment allowlist, `/` as working directory, and closed standard input, streams bounded stdout and stderr chunks over the authenticated channel, and reports a terminal status after reaping every descendant.
The retained command `/bin/busybox uname -a`, with a 10 s timeout and a 65,536-byte output allowance, returned `Exited(0)` with exactly these stdout bytes and empty stderr:

```text
Linux soma-4021a60cea3a 6.12.107-soma-v1 #1 SMP PREEMPT_DYNAMIC 2026-08-29T00:00:00Z x86_64 GNU/Linux
```

The hostname is the Instance-derived label installed during identity Repair.
The `node:22` Generation returned `Exited(0)` and stdout exactly `v22.23.2\n` from `/usr/local/bin/node --version` with a 30 s timeout.

### 3.7 Authenticated shutdown, exit, and cleanup

The host side sends an authenticated `Shutdown`.
`crates/soma-guest-agent/src/shutdown.rs` kills stray descendants, reaps orphans, syncs filesystems, acknowledges within a 5 s budget, prints `soma-guest-agent: shutdown acknowledged`, and calls `reboot(LINUX_REBOOT_CMD_RESTART)`.
The Machine has no ACPI and no paravirtual power-off, so a power-off request would degrade to `halt` and park the vCPU inside KVM invisibly; `reboot=k` turns restart into the keyboard-controller reset pulse that the Machine already treats as the orderly `Reset` exit.
`SandboxMachine::finish` then joins the vCPU thread within the exit deadline, stops the device thread, unregisters every ioeventfd and irqfd, retires the launch page if it was somehow still mapped, drops the VM, the KVM descriptor, and every mapping, and assembles the evidence.

The retained run recorded exit `Ok(Reset)`, launch page retired `true`, 4 descriptors before and 4 after, 2 threads before and 2 after, a peak of 6 threads and 36 descriptors while running, 156 MMIO reads and 187 MMIO writes with 0 transport violations and 0 queue-notify exits, and no device fault.
The EROFS root read back with the same SHA-256 after the run, and the private overlay head no longer matched the sterile template it was copied from.

### 3.8 The COLD timeline

The table below is copied exactly from [the first sandbox command evidence](../evidence/2026-08-29-x86_64-first-sandbox-command.md), busybox Generation, retained run.
It is a COLD timeline, one sample, debug build, inside the `ubuntu:24.04` container with `/dev/kvm` passed through that the evidence describes, on a busy Ubuntu 24.04 x86_64 development host with an Intel Core Ultra 9 275HX.
The clock starts when `SandboxMachine::create` begins, before the kernel and initramfs are read.
`RunStart` is vCPU 0 entering `KVM_RUN`.
It is not a benchmark, it is not a restore, and it must not be compared with any Ready or first-command objective.

| Milestone | ns since creation | delta ns | ns since `KVM_RUN` |
| --- | ---: | ---: | ---: |
| CreateVm | 23,173,405 | 23,173,405 | - |
| MapRegister | 23,173,704 | 299 | - |
| Platform | 23,259,464 | 85,760 | - |
| Devices | 23,293,678 | 34,214 | - |
| LaunchPageMapped | 24,788,085 | 1,494,407 | - |
| LoadGuest | 37,923,246 | 13,135,161 | - |
| Vcpu | 39,179,904 | 1,256,658 | - |
| Events | 39,231,502 | 51,598 | - |
| LaunchPageWritten | 39,305,623 | 74,121 | - |
| EventLoop | 39,401,210 | 95,587 | - |
| RunStart | 39,503,267 | 102,057 | 0 |
| KernelInit | 179,554,518 | 140,051,251 | 140,051,251 |
| LaunchPageConsumed | 220,037,351 | 40,482,833 | 180,534,084 |
| VsockConnected | 220,042,773 | 5,422 | 180,539,506 |
| Handshake | 229,860,678 | 9,817,905 | 190,357,411 |
| LaunchPageRetired | 231,006,481 | 1,145,803 | 191,503,214 |
| Ready | 232,097,391 | 1,090,910 | 192,594,124 |
| AgentReadyLine | 232,204,687 | 107,296 | 192,701,420 |
| Execute | 239,693,975 | 7,489,288 | 200,190,708 |
| Shutdown | 241,193,525 | 1,499,550 | 201,690,258 |
| GuestExit | 242,431,399 | 1,237,874 | 202,928,132 |
| Cleanup | 261,362,829 | 18,931,430 | 221,859,562 |

How to read it.
`CreateVm` includes reading the 21.5 MiB kernel and 1.6 MiB initramfs, 8.9 ms, and the capability probe, which creates and destroys a throwaway VM, 13.5 ms.
`KernelInit` is the Host timestamp of the console line `Run /init as init process`, and `AgentReadyLine` is the Host timestamp of `soma-guest-agent: ready`.
The immediately preceding run of the same test, with the same artifacts except the per-run key, reached `Ready` 163.7 ms after `KVM_RUN`, so the boot-to-Ready interval varied between 164 ms and 193 ms across the two samples on a loaded host.
The `node:22` run in the same evidence reached `Ready` 129,140,040 ns after `KVM_RUN`, and its `Execute` delta of 39,631,493 ns is the complete round trip of loading the Node binary from the 1.1 GB EROFS root and reporting `v22.23.2`.

Host residency in the same run, single sample per line from a 2 ms sampler thread, debug build: the test process peaked at `VmRSS` 46,800 kB while running, and in the last consistent sample the guest kernel had touched 32,168 kB of the 262,144 kB registered.
For the 1 GiB `node:22` Machine the guest mapping reached 67,468 kB of 1,048,576 kB registered.
These are diagnostics for one test process and not a per-Machine overhead figure.

### 3.9 The environment behind the numbers

The retained run happened inside an `ubuntu:24.04` container started with `--device /dev/kvm --user 1000:1000 --group-add 993 --security-opt seccomp=unconfined`, because the Host's interactive seat session ended mid-work and `systemd-logind` moved the `uaccess` ACL on `/dev/kvm` to the display-manager user.
The evidence states that the container adds no privilege beyond the device node, runs on the same Host kernel and KVM module, and is a host-access workaround rather than an isolation claim.
The Host was `Linux 7.0.0-30-generic` on Ubuntu 24.04.4 LTS with Rust `1.98.0`, a debug profile for the test process, and an `x86_64-unknown-linux-musl` release profile for the Guest agent.

## 4. What is designed, component-tested, or only historically live-proved

The design for the complete KVM sandbox is written down in the research documents and ADRs, and the decision map tracks every ticket.
The repository's own wording for what the cold-boot proof does not show, taken from [the first sandbox command evidence](../evidence/2026-08-29-x86_64-first-sandbox-command.md) and [the VMM decision map](../research/vmm-decision-map.md), is reproduced here so this guide cannot overstate it.

Snapshot capture and restore.
The cold-boot evidence says of its own run: "No snapshot capture or restore: this is a cold boot from the compiled artifacts; the snapshot codec was not exercised and no memory object was captured."
That sentence describes only that run.
`crates/soma-kvm/src/snapshot/` holds the `SOMASNP` v1 codec, compatibility check, and typed step orders, and `crates/soma-kvm/src/x86_64/snapshot/` turns them into KVM calls.
Capture and repeated restore are live-proved at `5d71524` on the current per-Instance authority design, whose object scan finds no Instance responder identity in `memory.raw`, `overlay.raw`, or `state.somasnap`. The earlier `7c1127d` run is retained as historical: it captured a Generation whose `memory.raw` still held a Generation-scoped responder private key, which ADR 0024 removed.
On current bytes capture and restore are therefore component-tested, and recapture is finding P1.5 of [the re-audit](../reviews/2026-08-29-implementation-reaudit.md).

Prepared workers and the VMM process.
"No prepared workers, allocator, or `soma-vmm` process topology: the test process drove the machine directly through crate-internal seams and `soma-guest`."
`crates/soma-vmm` is the provider-neutral lifecycle interface with an `UnavailablePlatform`, no crate depends on it, and [the jail evidence](../evidence/2026-08-29-vmm-jail-live.md) records that "the real `soma-vmm` binary: it does not exist yet."
Ticket #12 is component-tested behind launcher and broker seams.

Jail around the real VMM.
"No jail: the VMM process ran with the test's own privileges, no seccomp, namespace, or cgroup policy was applied, and the container used for the run is a host-access workaround, not an isolation claim."
The launcher "constrains the static `jail-probe` stand-in and does not yet wrap the real `soma-vmm` binary, transfer a TAP endpoint, or serve prepared workers."

Network attach to the Machine.
"No network egress: the network device sits behind the link-down loopback backend; the guest configured `eth0` and a default route, but no frame left the machine and no TAP or host network profile exists."
For the broker, "Proxy attachment, ingress forwarding, jailed VMM transfer, and virtio-net attach remain open." The daemon socket now authenticates its peer and gates every operation on a capability, and forwarding requires an activation receipt minted by a repaired authenticated guest session.

Certification.
Snapshot certification, promotion, and ready Generation re-verification are component-tested.
The Linux live proof that captures Node 22, installs its three objects, certifies the Candidate, promotes it, and re-verifies the resulting Generation is compiled as an ignored hardware test and still requires a fresh KVM-host run for current evidence.
Under ADR 0024 the Generation has no responder key to bind, and the Host holds the public half of the keypair it just sampled.

CLI and MCP on KVM.
`crates/soma-local/src/backend/kvm.rs` answers every resolve, launch, execute, inspect, and cleanup request with `BackendFailureKind::Unsupported`; only the capability probe behind `doctor` runs.
The public KVM Backend is therefore designed, not component-tested.

A Generation from a Template Lock.
The module map lists "a Generation built from a Template Lock" among the things the workspace does not yet contain, and tickets T6 through T18 of [the Template implementation map](../research/template-implementation-map.md) cover registry resolution, rootfs inspection, the build plan, Generation construction from a lock, publication, and remote resolution.

Latency.
"No latency claim: the numbers are unoptimized debug-build cold-boot single samples under host contention and inside a container, and must not be compared with a restored snapshot or any Ready or first-command objective."
The targets in the README performance contract are admission targets for a future certified engine, not measurements.

Repair after a clone.
The cold-boot evidence says of its own run: "The guest's own entropy, identity, and network repair effects were exercised once on a cold boot only; nothing here shows that a cloned Instance discards captured state."
The restore evidence at `5d71524` shows it on current code: two restored Instances each saw their own private write, neither saw the other's, and the shared memory object was unchanged under the private mapping.
That is a historical live-proof, so on current bytes clone repair is component-tested.

## 5. Host-side pieces that exist and what each proved

Three Linux crates implement Host-side mechanisms that the designed `soma-vmm` process will consume.
None of them is integrated: none is wired into the proven path in section 3.
Each has its own retained evidence, and each evidence file lists what it does not prove.

### 5.1 `soma-netd`: sterile network bundles

`crates/soma-netd` is the privileged network broker from [the Linux network profile](../research/linux-network-profile-v1.md).
It owns network namespaces, TAP and veth devices, `/30` IPAM, MAC derivation, nftables text, conntrack zones, resolver policy, port reservations, a durable ledger, single-use claimant-bound activation, ordered release, and reconciliation.
The VMM side receives exactly one TAP descriptor over `SOCK_SEQPACKET` with `SCM_RIGHTS` and a fixed typed header.

[The network profile evidence](../evidence/2026-08-29-linux-network-profile-live.md) records a run as a privileged process inside the pinned Ubuntu 24.04 container on the real Host kernel.
It proved:

- A sterile bundle's guest link is down and its namespace forwards nothing before activation.
- Assignment produced the exact `LaunchNetwork` values: address `10.200.0.2` with prefix 30, gateway `10.200.0.1`, resolver `1.1.1.1`, vsock CID 5, and generation 1.
- The TAP transfer delivered one descriptor with a matching header.
- After activation the gateway answered ARP and ICMP, a public TCP listener answered through masquerade, and the declared resolver answered.
- The cloud metadata address, an undeclared resolver, the Host address, a peer guest, and the peer's gateway all got silence.

Both bundles released completely, reconcile reported zero unowned objects, and one hundred prepare, assign, activate, release cycles left no namespace pin, link, or table behind.

Per-operation wall times of that 100-way burst, debug build, one thread, inside the container, with p99 being the largest of one hundred samples:

| Operation | p50 | p99 |
| --- | ---: | ---: |
| prepare | 30.0 ms | 46.4 ms |
| assign | 15.8 ms | 23.2 ms |
| activate | 3.5 ms | 4.7 ms |
| release | 55.3 ms | 77.1 ms |

`prepare` spawns `nft` twice and `release` spawns `conntrack` once and `nft` up to three times, so the version 1 subprocess mechanism dominates both, and the evidence names a netlink and libnftnl binding as the lever rather than a kernel limit.
It does not prove a jailed VMM, a virtio-net attach of the transferred TAP, traffic from a guest Linux kernel, IPv6, ingress forwarding, proxy attachment, a complete lifecycle over the daemon socket, crash recovery, or broker behavior with `CAP_NET_ADMIN` outside a container.

### 5.2 `soma-storage`: the prepared-head decision

`crates/soma-storage` implements [the XFS reflink storage profile](../research/xfs-reflink-profile.md): published overlay classes with exact-size admission, a profile probe that proves XFS with one working `FICLONE` before any head exists, sterile ext4 templates from a pinned formatter recipe, descriptor-only `FICLONE` head creation with `fsync` and `FIEMAP` shared-extent verification, a single-use head ledger, release, and reconciliation.

[The XFS reflink evidence](../evidence/2026-08-29-xfs-reflink-profile.md) is a decision input rather than a latency claim.
The five live conformance tests passed on a loop-backed XFS `reflink=1` filesystem inside a privileged pinned Ubuntu 24.04 container: the probe accepted the reflink mount and rejected a `reflink=0` mount without any copy fallback, two clones diverged without touching the template or each other, exhausting a clone reported `ENOSPC` while the template digest survived, and 32-way concurrent create and cleanup left a reconcilable directory.
The measurement matrix, release build, crossed 100 MiB, 1 GiB, and 4 GiB templates, sterile, preallocated, and fragmented extents, warm and cold cache, 1, 10, and 100 simultaneous clones, ten percent free space, and 100 concurrent unlinks: 69 cells with 200 raw samples each and zero failures.
The best 100-way cell has a complete-clone p99 of 9.9 ms and the worst 1,868 ms, against the 1.00 ms p99 disk share of fresh resource activation, and even the best single-clone cell is 1.25 ms at p99 because the durable file `fsync` alone costs 0.6 ms at p50 and 1.1 ms at p99.
The decision is that on-demand cloning is not admitted and prepared sterile heads are mandatory, created, synced, and verified outside Launch, with head destruction also off the request path because 100 concurrent unlinks raised the 100-way p99 from 21.5 ms to 57.1 ms.
The filesystem was a loop device over a sparse image file on the Host's ext4 NVMe root on a busy development machine, so the numbers say nothing about a raw partition or a certified Host class.
The proven path in section 3 does not use this crate: the test creates the private head with a plain file copy of the template in `crates/soma-kvm/tests/x86_64_sandbox_boot/generation.rs`.

### 5.3 `soma-jail`: the VMM launcher

`crates/soma-jail` implements [the VMM jail profile](../research/vmm-jail-profile.md): one ephemeral UID and GID, fresh user, mount, PID, network, IPC, and UTS namespaces, one cgroup v2 leaf with `memory.max`, `memory.swap.max=0`, `memory.oom.group=1`, `cpu.max`, and `pids.max`, a sealed descriptor table, an empty read-only tmpfs root entered through `pivot_root`, `no_new_privs`, hand-assembled classic BPF seccomp filters with no libseccomp dependency, `execveat` from an open descriptor, pidfd ownership, a ledger, and reconciliation.

[The jail evidence](../evidence/2026-08-29-vmm-jail-live.md) records fifteen live acceptance tests passing as root inside a privileged Ubuntu 24.04 container, because Ubuntu's AppArmor restriction blocks unprivileged user namespaces on the Host.
They proved:

- Only the manifest descriptors are visible, and injected descriptors fail closed before seccomp.
- The child runs as uid and gid 60001 with PID 1 inside six fresh namespaces with zero capabilities.
- Its root has zero entries and no procfs or sysfs.
- The cgroup limits read back exactly, and pids and memory exhaustion stay inside the leaf.
- `socket` and `execve` are `SIGSYS` kills recorded in the evidence.
- `KVM_GET_API_VERSION` on the transferred `/dev/kvm` returned 12 while `TUNSETIFF` on the same descriptor was killed.
- The steady-state filter drops setup-only syscalls while threads keep working.
- A stuck child dies through its pidfd, and the child dies with its launching thread and with its launcher process.
- A crashed launcher's leaf is recovered from the record alone.

The startup filter is 222 BPF instructions with fingerprint `0x40b7c33a9001c79b` and the steady-state filter 135 instructions with fingerprint `0xe748c586d5877538`, both pinned by a golden test.
The constrained program is the static musl `jail-probe` stand-in, so the measured syscall inventory is musl's plus the Rust runtime's, and a glibc-linked VMM would need its own trace.
It does not prove anything about the real `soma-vmm` binary, a transferred TAP endpoint, `io.max`, snapshot ioctls, a stuck `KVM_RUN`, prepared workers, or any latency objective.

## 6. How an agent or a human uses SOMA today

The usable lifecycle on `main` runs on the Docker Backend and the Apple Backend.
The KVM Backend is a capability probe plus the ignored test in section 3.

### 6.1 The CLI

`crates/soma-cli` builds the `soma` binary.
Its commands, from `crates/soma-cli/src/cli.rs`, are `run`, `machine launch`, `machine exec`, `machine inspect`, `machine stop`, `machine destroy`, `doctor [--strict]`, and `version`.
The global options are `--format human|json`, `--backend auto|macos|docker|kvm`, `--runtime PATH` for an explicit Apple container executable, and `--state-root PATH` for the durable state shared with `soma-mcp`.
Every run or launch carries a Machine shape through `--vcpus`, `--memory-mib`, and `--storage-mib`, with the defaults of 1 vCPU, 1,024 MiB, and 10,240 MiB.
The network policy is `--egress unspecified|denied|internet|unrestricted`, also spelled `--network`, `--dns unspecified|denied|system|custom`, a repeatable `--dns-server`, and a repeatable `--publish`, with egress and DNS both defaulting to denied.
`run` and `exec` also carry `--timeout-ms` and `--max-output-bytes`, whose defaults are 30,000 ms and 1,048,576 bytes in `crates/soma/src/request/execution_limits.rs`.
A command is an absolute guest executable and an argument vector after `--`; there is no shell string.

Backend selection in `crates/soma-local/src/backend/mod.rs` is fail-closed.
On Apple Silicon macOS, `auto` selects Docker when the Docker daemon answers and the Apple Backend otherwise.
On Linux x86_64, `auto` selects the KVM Backend, and the KVM Backend answers every lifecycle operation with the `unsupported_backend` error and a Receipt whose terminal status is `failed`, so a Linux user must select Docker explicitly.
On any other target the local engine fails closed with `UnsupportedTarget`, which the lifecycle commands report as `unsupported_backend` and `doctor` reports as `unsupported_target`, and it never runs the workload on the Host.

The commands below are the ones from the README and the evidence files.
Probe the selected Backend without overstating readiness:

```sh
cargo run --locked -p soma-cli -- doctor
cargo run --locked -p soma-cli -- --backend kvm doctor
```

[The halt-guest evidence](../evidence/2026-08-29-x86_64-kvm-halt-guest.md) records the KVM probe on the Ubuntu 24.04 host as `kvm-api-12-vcpu-mmap-12288` with `runtime-ready: yes` and `production-ready: no`.
Run one bounded command from an OCI image and prove cleanup on the Docker Backend:

```sh
cargo run --locked -p soma-cli -- --backend docker run node:22 -- /usr/local/bin/node --version
```

The Docker Backend in `crates/soma-local/src/backend/docker` pulls the image for the Host's own Linux architecture, records the observed manifest digest, and creates the container with `--read-only`, `--cap-drop ALL`, `--security-opt no-new-privileges`, `--pids-limit 256`, a 64 MiB tmpfs on `/tmp`, the requested `--cpus` and `--memory`, and one of exactly two network modes.
It uses `none` only for denied egress with a denied resolver and no published ports, and `bridge` only for unrestricted egress with the system resolver and no published ports; every other combination, including any `--publish`, fails closed as unsupported.
The unrestricted form is therefore `--backend docker run --egress unrestricted --dns system ...`.
The Backend keeps the container alive with `/bin/sh` as the entrypoint, so on this Backend the image must contain a shell even though the requested command is executed directly through `docker exec` and never through one; that is a development-Backend limitation and does not apply to the KVM path.
Its Receipt reports `docker_container`, `linux_container` isolation, on-demand preparation, and observed-only digest binding, and it reports writable storage as not verified.
[The Docker evidence](../evidence/2026-08-29-docker-node22-local.md) records five consecutive `node:22` one-shot runs on the development Mac returning `v22.23.2` with complete cleanup in approximately 1.19 s to 1.24 s each from acceptance through cleanup, with the launch milestone about 1.01 s after acceptance because the path invokes the Docker CLI and resolves the image on demand.
That is a container boundary inside Docker's Linux VM, not a per-sandbox hardware VM.

The Apple Backend exists only on Apple Silicon macOS and creates one Linux VM per Instance through the pinned Apple container runtime:

```sh
cargo run --locked -q -p soma-cli -- doctor --strict
cargo run --locked -q -p soma-cli -- --format json run node:22 -- /usr/local/bin/node --version
```

[The Apple evidence](../evidence/2026-08-29-apple-node22-one-shot.md) records the strict probe passing as runtime-ready but not production-ready, the command returning exactly `v22.23.2\n`, the Backend observing one vCPU and 1,024 MiB but not verifying storage, hardware-VM isolation and on-demand preparation as basic Backend observations, a complete accepted-request through cleanup boundary of 1.995378542 s, and a machine-launched through command-ready interval of 17.228458 ms.
Digest binding was observed-only because Apple container 1.3 cannot launch the local image by an immutable digest reference.

Managed Machines use the same Backends and a durable file-backed state store:

```sh
cargo run --locked -p soma-cli -- --backend docker machine launch node:22
cargo run --locked -p soma-cli -- --backend docker machine exec --instance-id <id> -- /usr/local/bin/node --version
cargo run --locked -p soma-cli -- --backend docker machine inspect --instance-id <id>
cargo run --locked -p soma-cli -- --backend docker machine stop --instance-id <id>
cargo run --locked -p soma-cli -- --backend docker machine destroy --instance-id <id>
```

The Instance identity is the 32-character lowercase value printed by `launch`, or one supplied with `--instance-id`; a Machine name given with `--name` is metadata only and never selects or authorizes an Instance.

### 6.2 The MCP server

`crates/soma-mcp` builds the `soma-mcp` binary, a stdio Model Context Protocol server.
`crates/soma-mcp/src/main.rs` wires `LocalToolRuntime`, which opens the same `soma-local` runtime and state store as the CLI for every tool call.
The tools are `soma_doctor`, `soma_run`, `soma_launch`, `soma_exec`, `soma_inspect`, `soma_stop`, and `soma_destroy`, with the same Machine shape, direct-command, network-policy, and identity contracts as the CLI, documented in [the agent integration guide](../integrations/agents.md).
The server reserves stdout for JSON-RPC, terminates a session on an inbound message above 8 MiB, and admits at most 32 concurrent tool executions.

Build and register it with the commands from that guide:

```sh
cargo build --release -p soma-mcp
claude mcp add --scope user soma -- /absolute/path/to/soma-mcp
codex mcp add soma -- /absolute/path/to/soma-mcp
hermes mcp add soma --command /absolute/path/to/soma-mcp
```

The `backend` tool input accepts `auto`, `local`, `kvm`, `macos`, and `docker` and follows the same fail-closed selection as the CLI, so a KVM selection can only answer `soma_doctor` today.

### 6.3 The KVM path, test-only

There is no CLI or MCP command that boots a Generation on KVM.
The proven path is the ignored test, which needs a readable and writable `/dev/kvm`, the pinned kernel from `kernel/build.sh`, its configuration text, the pinned erofs-utils 1.9.4 directory, the static Guest agent from `scripts/build-guest-agent.sh`, the ext4 formatter from e2fsprogs 1.47.0, and Docker for `docker save`:

```sh
SOMA_EROFS_TOOLS=/absolute/path/to/erofs-utils-1.9.4 \
  cargo test --locked -p soma-kvm --test x86_64_sandbox_boot -- --ignored --test-threads=1 --nocapture
```

Cargo runs an integration test from the package root, so a relative path in any of the test's path variables resolves inside `crates/soma-kvm` rather than the repository root.
Write absolute paths, or leave the kernel and agent variables unset and let their defaults find the artifacts.
With `SOMA_GUEST_AGENT` unset the test looks for `target/x86_64-unknown-linux-musl/release/soma-guest-agent` under the workspace root, which is exactly what `scripts/build-guest-agent.sh` writes.
With `SOMA_X86_64_VMLINUX` unset the test scans `kernel/out` under the workspace root and polls for a stable `vmlinux-<ver>-soma-v1` for up to 45 minutes before it fails.
Both variables may name absolute paths to override those defaults.
`SOMA_OCI_BUSYBOX_LAYOUT` and `SOMA_OCI_NODE_LAYOUT` may name pre-exported OCI layouts.
On a host where the `docker` CLI is rootless Podman, the tests' own `docker save` export does not produce the layout they expect; export the layouts once with `podman save --format oci-dir -o <dir> <image>` and pass them through those two variables.
The same host class needs no accommodation for `kernel/build.sh`, which detects the Podman CLI and keeps the caller's uid inside the builder on its own.
The test fails explicitly when a prerequisite is missing, including when the `node:22` image cannot be exported: these tests are `#[ignore]`d, so a run that reaches them asked for them by name, and a missing prerequisite is a failed run rather than a test that reports `ok` having executed nothing.

### Scratch space

Each live run compiles its own Generation into `target/tmp/`, which for `node:22` is an EROFS root near 1.1 GiB plus an overlay head and a snapshot, and trees are reclaimed only once they are six hours old.
Repeated runs, and runs across several worktrees, therefore accumulate tens of gigabytes quickly.

An exhausted filesystem does not announce itself. It surfaces as `CompileError { phase: FormatRoot, kind: Toolchain }` when the root formatter cannot write, and as `MachineError { phase: Run, kind: Timeout }` with `launch_page_retired=false` when a partly written tree strands the boot until its deadline. Both read as a flaky live test, and both can fail a different test on each run.

`scratch_dir` now measures free space before a run starts and fails naming the real condition, so check that message before forming any timing hypothesis. Reclaim by deleting `target/tmp` in each worktree; it is regenerated on the next run.

Never wrap `docker run` in `timeout` when driving these tests: it kills the client and leaves the container running, which keeps consuming a core and writing scratch. Name the container and stop it by name instead.

## 7. Where to read next

- [CONTEXT.md](../../CONTEXT.md) and [GLOSSARY.md](../../GLOSSARY.md) for the product vocabulary this guide builds on, which does not include the machine terms listed at the top of this guide.
- [Beginner architecture guide](../architecture/beginners-guide.md) for the objects and layers, and [What makes one SOMA sandbox](../architecture/sandbox-stack.md) for the containment and dependency map.
- [SOMA visual atlas](../architecture/visual-atlas.md) for the machine, filesystem, workload, and capacity pictures.
- [Creating a Template](creating-templates.md), [SOMA template system](../architecture/template-system.md), [ADR 0022](../adr/0022-compose-templates-into-generation-locks.md), and [the Template implementation map](../research/template-implementation-map.md) for the Template plane.
- [Generation compiler v1](../research/generation-compiler.md), [x86_64 machine contract v1](../research/x86_64-machine-contract.md), [minimal device surface v1](../research/minimal-device-surface.md), [Linux guest integration v1](../research/linux-guest-agent-integration.md), and [snapshot format v2](../research/snapshot-format-v2.md) for the Machine design.
- [VMM jail profile](../research/vmm-jail-profile.md), [Linux network profile v1](../research/linux-network-profile-v1.md), [XFS reflink profile](../research/xfs-reflink-profile.md), and [prepared worker protocol](../research/prepared-worker-protocol.md) for the Host-side design.
- [VMM decision map](../research/vmm-decision-map.md) for the status of every ticket, and [Linux VMM handoff](../operations/linux-vmm-handoff.md) for the implementation order.
- Every `docs/evidence/2026-08-29-*.md` file for the retained results, each with its own evidence boundary and nonclaims.
- [Module map](../architecture/module-map.md) for crate ownership, and [the agent integration guide](../integrations/agents.md) for the MCP contract.
- [Threat model](../threat-model.md), [benchmark contract](../benchmark-contract.md), and [ROADMAP.md](../../ROADMAP.md) before evaluating any trust or performance claim.
