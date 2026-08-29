# SOMA minimal device surface v1

## Decision

SOMA machine contract v1 exposes five modern virtio 1.0-or-later devices over fixed virtio-mmio version 2 transports.
The devices are one immutable EROFS root block device, one Instance-private writable overlay block device, one network device, one vsock device, and one entropy device.
Authenticated control and orderly shutdown share the vsock device and do not create another virtual device.

Version 1 has no PCI or PCIe bus, device enumeration, hotplug, MSI, MSI-X, IOMMU, packed virtqueues, vhost backend, virtio console, balloon, memory device, filesystem device, SCSI controller, or generic user-configurable device.
The diagnostic 16550 UART belongs only to cold-boot certification and is absent from a production Generation snapshot.

This is the complete device surface required before decision-map ticket #6 can compile a bootable Generation.
It is a design contract and not evidence that the devices have been implemented or benchmarked.

## Primary sources

- The [Virtio 1.4 specification](https://docs.oasis-open.org/virtio/virtio/v1.4/virtio-v1.4.html) defines the modern MMIO transport, split virtqueues, feature negotiation, reset, block, network, entropy, and socket devices.
- The [Linux virtio-mmio driver](https://github.com/torvalds/linux/blob/master/drivers/virtio/virtio_mmio.c) defines command-line discovery and requires `VIRTIO_F_VERSION_1` for a version 2 transport.
- The [Linux kernel parameter reference](https://www.kernel.org/doc/html/latest/admin-guide/kernel-parameters.html) defines `virtio_mmio.device=<size>@<baseaddr>:<irq>[:<id>]` and permits one declaration per device.
- The [rust-vmm vm-virtio workspace](https://github.com/rust-vmm/vm-virtio) provides modern-only device and queue components but leaves backend behavior and event handling to the VMM.
- The [rust-vmm virtio-queue contract](https://github.com/rust-vmm/vm-virtio/blob/main/virtio-queue/README.md) supports split queues, serializable queue state, notification suppression, and explicit device-side buffer validation.
- The [Firecracker kernel policy](https://github.com/firecracker-microvm/firecracker/blob/main/docs/kernel-policy.md) confirms that a production microVM VMM can omit PCI and expose virtio devices through MMIO.
- The [Dragonball virtio-blk advisory](https://github.com/kata-containers/kata-containers/security/advisories/GHSA-fgm4-mv68-h344) is direct evidence that unchecked guest-controlled I/O lengths can become a guest-to-host escape.

## Transport choice

### Selected: modern virtio-mmio version 2

Each transport occupies one fixed 4 KiB MMIO page and one dedicated interrupt.
The guest discovers every page through a SOMA-owned kernel command-line declaration.
The transport reports magic `0x74726976`, transport version `2`, a fixed device identifier, and `VIRTIO_F_VERSION_1`.
Legacy virtio-mmio version 1 is rejected.

The selected transport has a small fixed register file and can notify a queue through one MMIO write.
SOMA registers the queue-notify address with `KVM_IOEVENTFD`, so ordinary guest notifications do not require userspace to decode an MMIO exit.
SOMA registers each device interrupt eventfd with `KVM_IRQFD`, so device completion does not require a userspace interrupt-injection ioctl on the steady-state path.

### Rejected for version 1: virtio-pci

PCI would require a bus and configuration-space model, enumeration, BAR placement, capability lists, INTx or MSI/MSI-X state, and more compatibility and snapshot fields.
Those mechanisms are useful for general-purpose machines, hotplug, and high queue counts, but none is required for SOMA's first one-vCPU sandbox.
Adding them would contradict the version 1 machine contract without reducing the number of guest-visible functions.

PCI may be introduced only as a different machine-contract version with its own boot, snapshot, hostile-input, and performance evidence.
It cannot silently replace MMIO for an existing Generation.

## Fixed topology

The version 1 machine limits RAM to 3 GiB, so the MMIO window begins at `0xd0000000` and cannot overlap RAM.
Each address, interrupt, device identifier, queue count, queue limit, feature allowlist, and command-line declaration is part of `GenerationId`.

| Slot | MMIO range | GSI | Device | Virtio ID | Queues |
| ---: | --- | ---: | --- | ---: | --- |
| 0 | `0xd0000000` to `0xd0000fff` | 5 | Immutable root block | 2 | request: 256 |
| 1 | `0xd0001000` to `0xd0001fff` | 6 | Writable overlay block | 2 | request: 256 |
| 2 | `0xd0002000` to `0xd0002fff` | 7 | Network | 1 | receive: 256, transmit: 256 |
| 3 | `0xd0003000` to `0xd0003fff` | 8 | Vsock control | 19 | receive: 256, transmit: 256, event: 64 |
| 4 | `0xd0004000` to `0xd0004fff` | 9 | Entropy | 4 | request: 64 |

The fixed guest command-line fragment is:

```text
virtio_mmio.device=4K@0xd0000000:5:0 virtio_mmio.device=4K@0xd0001000:6:1 virtio_mmio.device=4K@0xd0002000:7:2 virtio_mmio.device=4K@0xd0003000:8:3 virtio_mmio.device=4K@0xd0004000:9:4
```

Every interrupt is a distinct edge-triggered IOAPIC route in version 1.
No device shares an interrupt.
The in-kernel irqchip and routes exist before any irqfd is registered, and every irqfd exists before the vCPU can run.

## Common virtio contract

### Feature negotiation

SOMA exposes an explicit per-device feature allowlist rather than forwarding a backend or host feature set.
Every device exposes `VIRTIO_F_VERSION_1`.
Version 1 uses split virtqueues and does not expose packed-ring, notification-data, ring-reset, access-platform, in-order, or shared-memory-region capabilities.
Indirect descriptors and event-index notification suppression remain disabled until dedicated conformance, fuzz, snapshot, and performance tests justify them.
Unknown or non-allowlisted driver feature bits make `FEATURES_OK` fail.

The negotiated feature words are captured in the Generation snapshot.
Restore requires exact equality between the captured allowlist, implementation device version, and restored negotiated value.
SOMA never renegotiates a captured device after restore.

### Queue invariants

Every queue limit above is a maximum and a power of two.
The driver-selected size must be nonzero, a power of two, and no larger than that queue's maximum.
Descriptor, available-ring, and used-ring regions must be aligned as required by the specification, must use checked arithmetic, must lie wholly in registered guest RAM, and must not overlap reserved boot, launch-page, or MMIO ranges.

Queue notification indexes outside the device's fixed queue count are ignored and counted as protocol violations.
A queue cannot activate twice without a complete device reset.
No queue work is accepted before the driver has acknowledged the device, negotiated features, set `DRIVER_OK`, and supplied a valid ready queue.

SOMA bounds one notification dispatch by the number of newly available heads and by a host-configured work budget.
More work is rescheduled through the event loop rather than monopolizing the sole device thread.
The event loop never follows an unbounded guest chain, recursively parses descriptors, or waits synchronously for guest progress.

### Hostile descriptor validation

Guest RAM and every descriptor are hostile even when KVM created the VM successfully.
For every access, the implementation must:

1. Read descriptor metadata through the guest-memory abstraction rather than dereferencing a guest address.
2. Reject an index outside the negotiated queue size.
3. Reject a chain longer than the negotiated queue size or any repeated descriptor index.
4. Reject address-plus-length overflow and any byte outside a registered guest-memory region.
5. Enforce device-readable and device-writable direction before touching a buffer.
6. Revalidate the actual buffer access instead of trusting only an earlier queue-layout check.
7. Reject unsupported indirect, packed, or nested descriptor forms.
8. Cap aggregate descriptors and bytes before allocating host memory or issuing host I/O.
9. Use checked conversions between guest integers, host offsets, `usize`, and backend API lengths.
10. Write a used length no larger than the validated writable capacity.
11. Treat malformed work as an individual request failure when the specification permits it, otherwise set `DEVICE_NEEDS_RESET` and stop the device.
12. Record only bounded counters and classifications, never guest buffer contents or authority material.

No guest-provided address, length, sector, packet field, port, CID, queue index, or feature bit reaches a host syscall before its device-specific validation succeeds.

### Interrupt behavior

Queue completion sets the used-buffer interrupt status before signaling the device irqfd.
Configuration changes set the configuration-change status before signaling the same device irqfd.
The guest acknowledges only bits it observed, and the transport clears exactly those bits atomically.
Notification suppression is honored only with the base split-ring flags supported by version 1.

The device event loop drains eventfds, processes bounded work, publishes used entries with the required memory ordering, decides whether notification is required, and then signals the irqfd.
Snapshot capture quiesces backends and queue processing before reading interrupt status.
Restore recreates eventfds and routes before a pending captured interrupt can be replayed.

### Reset behavior

A status write of zero performs a complete transport and device reset.
Reset stops new backend work, drains or cancels owned host operations, removes queue event registrations, clears negotiated features and queue selection, clears interrupt status, and returns the device to its unactivated state.
Reset cannot close or replace another Instance's backing resources.

The VMM process is single-use, so reset is a guest protocol operation rather than a tenant-reuse mechanism.
Destroy still closes every backend and descriptor even if reset did not complete.

## Block devices

The guest sees one read-only raw block device containing the deterministic EROFS Generation root and one writable raw block device containing an Instance-private ext4 overlay.
The immutable EROFS base is shared read-only and is never opened writable by `soma-vmm`.
The overlay is a fresh private copy-on-write head cloned from a sterile filesystem image before assignment, and no two Instances share a writable head.
The pinned guest init mounts EROFS read-only, mounts the private ext4 filesystem as the OverlayFS upper and work storage, and pivots to the combined writable root before starting the guest agent.

Each block device exposes one request queue of at most 256 entries.
The root device exposes `VIRTIO_BLK_F_RO` and `VIRTIO_BLK_F_BLK_SIZE` in addition to the common modern feature.
The overlay device exposes `VIRTIO_BLK_F_BLK_SIZE` and `VIRTIO_BLK_F_FLUSH` in addition to the common modern feature.
The logical block size is fixed in the Generation manifest.
Discard, write-zeroes, secure erase, SCSI passthrough, multiqueue, topology, geometry, writeback-cache negotiation, and host-dependent features are absent.

Each request must contain one complete fixed-size header, a directionally valid bounded data region when the request type requires one, and one writable status byte.
The implementation rejects unsupported request types, invalid descriptor direction, sector multiplication overflow, offset-plus-length overflow, non-block-aligned data, requests beyond the certified virtual capacity, an aggregate length above the fixed request limit, and host short I/O.
Guest-controlled lengths are never used directly to construct an `io_uring` operation or host buffer.

Read and write completion report only after the host operation finishes.
Flush completion reports only after the private head satisfies the selected durability policy.
The guest filesystem must issue a flush at the Generation quiesce boundary before snapshot capture.

Snapshot state contains both transports, negotiated features, queue configuration and cursors, interrupt status, configuration generation, capacities, logical block sizes, private-head identity, immutable-root digest, sterile-overlay digest, and the durability boundary.
No host file descriptor number or host path enters the snapshot.
Restore opens and verifies the immutable root and assigned private overlay, proves their identities and sizes match, installs both backends, restores both queue states, registers queue ioeventfds and irqfds, and only then permits vCPU resume.

## Network device

The guest sees one virtio network interface connected to one already-open TAP descriptor supplied by the privileged network broker.
Version 1 exposes one receive queue and one transmit queue, each with at most 256 entries.
It exposes only `VIRTIO_NET_F_MAC` in addition to the common modern feature.
The control queue, multiqueue, mergeable receive buffers, checksum offload, segmentation offload, receive hash, standby, and guest-announcement features are absent.

The Generation contains a placeholder MAC, but Instance repair installs or confirms the effective unique network identity before Ready.
The device never opens `/dev/net/tun`, changes routes, creates namespaces, or applies firewall rules itself.

Transmit validates the virtio header length, descriptor direction, complete Ethernet-frame length, aggregate chain length, and configured maximum frame size before writing to TAP.
Receive reserves a validated writable guest chain before reading a bounded frame from TAP and never writes beyond that capacity.
Short, oversized, malformed, or policy-incompatible frames are dropped with bounded counters.
The host network profile, not the guest, enforces egress, metadata, proxy, and ingress policy.

Snapshot state contains transport state, negotiated features, both queue states, interrupt status, configuration generation, placeholder MAC, and link status.
It contains no TAP descriptor, host interface name, network-namespace name, lease, IP address, firewall handle, or live packet.
Capture requires empty host-side device buffers and no partially consumed descriptor chain.
Restore attaches the fresh TAP, resets all transient packet state, restores queues while link remains down, completes authenticated network repair, and raises link only when the effective network is safe to activate.

## Vsock control device

Vsock is the sole production guest-control transport.
It carries the already-defined bounded application protocol and Noise-authenticated session, including repair, execute, output, terminal result, and orderly-shutdown messages.
Vsock transport identity is not authentication.

Version 1 exposes receive and transmit queues of at most 256 entries and an event queue of at most 64 entries.
It exposes no device-specific optional feature bits.
The host endpoint accepts only the fixed SOMA control port, and the guest context identifier is assigned from an operator-owned collision-free range.
CID, port, operation identity, Generation identity, and Noise session identity are independent fields.

Packet parsing checks the complete fixed header, little-endian fields, source and destination CID, allowed port, socket type, operation, flags, declared payload length, descriptor capacity, and protocol message bound before allocation or copy.
Unsupported packet types, impossible credit accounting, arithmetic overflow, stale connection generations, and packets from a captured session are rejected.
Credit updates are bounded and cannot wrap counters or authorize bytes beyond actual queue capacity.

Snapshot state contains transport state, negotiated features, three queue states, interrupt status, configuration generation, and the captured CID placeholder.
It contains no open host socket, connection, credit window, unread packet, Noise key, session key, launch secret, or live authority.
Capture occurs only at the guest agent's disconnected repair point.
Restore assigns the fresh CID, clears all connection and credit state, attaches a fresh host endpoint, restores empty queues, and allows only a new authenticated repair session.

An authenticated shutdown request asks the guest agent to sync filesystems and invoke orderly poweroff.
The host lifecycle retains an independent bounded stop deadline and can force vCPU exit and process destruction.
No virtual keyboard, power button, ACPI controller, or separate shutdown device is required.

## Entropy device

The guest sees one virtio entropy device with one queue of at most 64 entries.
It exposes no device-specific optional feature bits.
The backend uses a fresh nonblocking operating-system CSPRNG source owned by the assigned Instance and never replays bytes from snapshot state.

Each request must provide one or more writable descriptors whose checked aggregate length does not exceed the fixed entropy-request limit.
The backend fills exactly the reported used length, never logs bytes, never accepts guest input as entropy, and treats host entropy failure as a launch failure before Ready.
Host randomness is used only through a reviewed CSPRNG interface and not through deterministic benchmark fixtures in production.

Snapshot state contains only transport state, negotiated features, queue state, interrupt status, and configuration generation.
It contains no random bytes, host RNG handle, buffered entropy, or deterministic generator state.
Restore attaches a fresh backend and makes it available before authenticated repair begins.
The guest's entropy repair remains mandatory because a device alone does not prove that every cloned user-space generator discarded captured state.

## Snapshot quiesce contract

Device capture is permitted only at the certified guest repair point.
At that point:

- Both block queues have no in-flight request and the writable overlay's required durability boundary has completed.
- The network queues have no partially consumed chain and host packet buffers are empty.
- Vsock has no connection, credit state, unread packet, or authenticated session.
- The entropy queue has no in-flight request or buffered random bytes.
- The event loop has drained queue notifications and backend completions.
- Interrupt status and pending IOAPIC state are stable and captured consistently.

Failure to prove any condition aborts Generation construction.
The builder never captures first and attempts to infer later whether device state was safe.

## Restore order

The device portion of restore follows this exact order:

1. Verify the Generation, machine-contract, device-contract, feature-set, queue-limit, and artifact identities.
2. Create the VM, private memory mapping, memory slots, irqchip, and fixed interrupt routes.
3. Open and verify the immutable root disk, private overlay head, fresh TAP, fresh vsock endpoint, and fresh entropy backend without exposing them to the guest.
4. Construct all five transports and device models at their fixed MMIO addresses.
5. Restore configuration, negotiated features, queue geometry, queue cursors, and captured interrupt status through validated state constructors.
6. Register queue ioeventfds and device irqfds.
7. Restore the IOAPIC and other machine interrupt state consistently with the captured device interrupt bits.
8. Retire every captured authority and install fresh Instance launch material through the separate launch page.
9. Enable backend event processing while network link and public ingress remain inactive.
10. Restore vCPU state and resume the vCPU.
11. Complete fresh vsock authentication, identity, entropy, time, and network repair.
12. Activate the safe network link and require the first bounded command before Ready.

Any failure unwinds backend, event, route, device, KVM, and memory ownership in reverse order.
No partial restore falls back to cold boot, Docker, Apple virtualization, or an unauthenticated transport.

## Implementation seams

The implementation should remain a set of narrow modules rather than one device-manager file.
The intended seams are:

```text
soma-kvm
  machine/x86_64        fixed addresses, GSIs, routing, KVM ordering
  bus/mmio              checked interval dispatch only
  virtio/transport      modern MMIO registers, status, features, reset
  virtio/queue          split-ring state, bounded descriptor access
  devices/block         request parser plus immutable and private backends
  devices/net           frame parser plus preopened TAP backend
  devices/vsock         packet and credit parser plus control endpoint
  devices/rng           bounded fill plus fresh CSPRNG backend
  events                ioeventfd, irqfd, epoll, bounded dispatch
  snapshot/device       versioned state validation and restore ordering
```

The common transport owns no device-specific parser.
The queue module owns no host backend.
Device parsers accept bounded guest-memory views and return typed operations.
Backends accept only already-validated operations.
Snapshot decoding constructs validated state values before mutating a live VM.

Reusing rust-vmm components does not transfer these responsibilities to rust-vmm.
SOMA still owns the feature policy, backend safety, event loop, device lifecycle, snapshot compatibility, and end-to-end hostile-input evidence.

## Required evidence

The device surface is implementation-complete only after Linux x86_64 tests prove:

1. The pinned guest discovers exactly five modern MMIO devices and no PCI bus.
2. The EROFS root and ext4 upper compose into a writable root, read a known Generation file, write privately, flush, and leave both shared base artifacts unchanged.
3. Network transmit and receive work through the supplied TAP while denied traffic remains denied by the host profile.
4. A fresh vsock session completes authenticated repair, one command, and orderly shutdown.
5. Entropy requests return fresh bytes across at least two restores of the same snapshot.
6. Every malformed queue, descriptor, request, frame, packet, feature, reset, and restored state class fails without host memory corruption, unbounded work, or cross-Instance access.
7. Fuzz targets cover each parser and the common queue and transport state machines.
8. Snapshot restore preserves valid queue and interrupt behavior while excluding transient I/O and authority.
9. Forced timeout and process crash release every descriptor, TAP, disk head, socket, eventfd, irqfd, memory mapping, and KVM object.
10. Raw latency samples demonstrate that device reconstruction and attachment remain inside the certified restore budget.

Until those results exist, SOMA must describe this document as the selected device contract rather than a working or faster device implementation.

## Implementation status

Items 1, 2, and 4 have live x86_64 evidence from a cold boot of a compiled Generation: the pinned guest registered exactly the five modern MMIO devices and no PCI bus, composed the EROFS root and the private ext4 upper into a writable root, executed a Generation file, changed only the private head, and completed one authenticated vsock session with repair, one command, and orderly shutdown.
The entropy device served the guest's `/dev/hwrng` reseed once, which is not the two-restore proof of item 5, and the network device has run only behind the link-down loopback backend, so item 3 is untouched.
Items 6 through 10 remain open, and the retained result is [the first sandbox command evidence](../evidence/2026-08-29-x86_64-first-sandbox-command.md).
