# Linux custom VMM handoff

This is the handoff for continuing SOMA's custom Rust VMM on a Linux host.
Use the [custom VMM decision map](../research/vmm-decision-map.md) as the canonical dependency order for research and implementation tickets.

## Current repository state

The current main branch is commit `08bf75e2439e788a6fea3a722fbb328860dc56a8`.
The working tree on the Mac was clean when this handoff was written.

The repository contains two working Mac development backends.

- `macos` creates a Linux VM through the pinned Apple Container runtime.
- `docker` creates a constrained Linux container through Docker Desktop.

The repository also contains the initial `soma-kvm` machine owner.
It opens KVM, creates a VM, and creates vCPU descriptors on Linux.
It does not yet boot a guest or expose a complete sandbox lifecycle.

## What was proven on the Mac

The Apple backend created and deleted an Ubuntu 22.04 Linux VM and executed a command inside it.
The cached end-to-end time was approximately 1.78 seconds.

The Docker backend created and deleted Ubuntu and Node 22 containers.
Five cached Node 22 runs took approximately 1.19 to 1.24 seconds end to end.

The Mac has Apple hardware virtualization support but no `/dev/kvm`.
The custom KVM backend therefore fails closed on the Mac with `unsupported_target`.

## Linux host prerequisites

The first target is Ubuntu 24.04 x86_64 on a bare-metal or KVM-capable host.

Verify the exact host before runtime work:

```sh
uname -a
uname -m
test -r /dev/kvm && test -w /dev/kvm
stat /dev/kvm
grep -E 'vmx|svm' /proc/cpuinfo | head
```

Then validate SOMA's capability probe:

```sh
cargo run --locked -p soma-cli -- --backend kvm doctor
cargo test --locked -p soma-kvm --lib
```

A passing probe proves KVM access only.
It does not prove guest boot, isolation, readiness, networking, or latency.

## Implementation order

Implement one vertical slice at a time.

1. Add page-aligned guest RAM allocation and one KVM user-memory slot.
2. Add x86_64 vCPU bootstrap state and a minimal test guest that halts.
3. Add a pinned Linux kernel and initramfs fixture.
4. Add serial or virtio-console output for boot diagnostics.
5. Add virtio-vsock or an equivalent private control channel.
6. Boot the SOMA guest agent and authenticate its handshake.
7. Add the first bounded direct command through the guest agent.
8. Add a read-only base disk plus private copy-on-write writable state.
9. Add per-instance network namespace, TAP, and explicit egress policy.
10. Wire the real lifecycle into the portable `soma-local` backend.
11. Add Linux integration tests that require `/dev/kvm` and are ignored elsewhere.
12. Measure cold boot, restore, readiness, first command, and cleanup separately.

Do not add snapshot or warm-pool claims until the guest reaches authenticated readiness and the first command succeeds.

## Expected first Linux failure

The current `soma-kvm` foundation may compile successfully but the portable local backend still returns an unsupported result for a complete KVM lifecycle.
That is expected until the guest boot and command path is wired.

Do not change the failure into a host-process fallback.
Do not label a capability probe as a working sandbox.

## Performance target

The approximately 10 ms target applies only to a prepared, cached Generation restored through a warm snapshot or equivalent copy-on-write path.
It does not apply to image download, OCI conversion, first-time disk creation, or cold Linux boot.

Record these measurements independently:

- image preparation
- cold VM boot
- warm restore
- restore through authenticated guest readiness
- restore through the first successful command
- cleanup

## Safety rules

- Keep the public SOMA request and receipt contracts stable.
- Keep KVM and device code inside focused Linux-only modules.
- Never silently downgrade KVM requests to Docker or a host process.
- Keep the guest agent and control channel authenticated before exposing `Ready`.
- Make every owned resource part of rollback and cleanup evidence.
- Do not publish benchmark numbers until the test includes a successful first command.
