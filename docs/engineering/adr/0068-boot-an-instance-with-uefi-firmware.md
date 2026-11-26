# 0068 - Boot an Instance with UEFI firmware

## Context

The owner wants Windows desktops to run as SOMA Instances, behind the same Instance interface as Linux Instances: create, stop, start, an always-attached disk, ports, and a screen.
Every machine contract so far (versions 1 to 6) boots a Linux kernel by PVH direct entry, with virtio-mmio devices declared on the kernel command line, ACPI and PCI off, a launch page for secrets, and the guest agent in the initramfs.
Windows needs UEFI firmware, ACPI, PCI devices, and a TPM, so none of those contracts can boot it.
Version 1 of the machine contract research note excluded BIOS and UEFI, and MISSION.md listed them as non-goals.

In a lab on an Ubuntu 24.04 KVM host, a Windows 11 disk booted under Cloud Hypervisor with the CLOUDHV.fd UEFI firmware, a software TPM, a virtio PCI disk and network, and a serial console.
The guest ran the MIOSA envd agent as a Windows service, answered exec, file and desktop requests, and kept a file across an ACPI power-off and a new monitor process.
The boot needed the Hyper-V enlightenments, virtio drivers bound to modern-only devices, Emergency Management Services on the serial port, and hibernation and Fast Startup off.

## Decision

Firmware is a property of the machine contract, not of a launch request.
Every Instance of a Generation boots the same way, so the choice is fixed when the Generation is built and verified, like every other machine property.

1. A new contract family, UEFI, starts at version 1001 so the PVH versions stay one ordered line.
   It is its own type, `UefiContract`, beside `MachineContract`, and `Firmware::of_contract_version` names the family a version belongs to.
   `MachineContract` keeps meaning a PVH machine, so every PVH boot, snapshot and restore path refuses a UEFI version as an unknown contract, failing closed.
   The version 1001 statement fixes ACPI on, PCI virtio block and network, a TPM 2.0, one serial port, the Hyper-V enlightenments, and no kernel command line.
2. A UEFI Generation binds a bootable disk image instead of a kernel, initramfs, root tree and overlay.
   Its root binding names the disk image, its kernel and initramfs bindings are empty, and its command line is empty.
   Verification refuses any other combination, so a PVH Generation cannot be launched as UEFI or the reverse.
   The disk image becomes the Instance's always-attached durable volume on first start.
3. The machine-host process keeps one process per Instance.
   For a UEFI contract its machine source runs Cloud Hypervisor as a child under the same jail profile model, with the firmware blob named by digest.
   The Instance socket, session, tunnels, ledger entries and capacity accounting stay as they are.
   A host-wide backend switch was rejected because one host must run PVH and UEFI Instances side by side.
4. Each Instance has its own software TPM whose state lives on its durable volume, so it follows the disk across stop, start and placement.
5. Cloud Hypervisor exposes virtio-vsock as a Unix socket.
   An adapter carries the existing authenticated guest session over it, so exec, files, terminals and port tunnels are unchanged.
   Secrets that PVH machines read from the launch page arrive as the session's first authenticated message instead.
6. The Windows guest side of that session is the MIOSA envd agent, which already runs as a Windows service.
7. Stop is an ACPI power-button request through the Cloud Hypervisor API, then a forced stop after a bounded wait.
   Create, start, delete and port exposure are the same operations as for PVH Instances.
8. Pause, checkpoint, restore, fork, suspend, prewarm, memory hotplug and resize answer `Unsupported` for UEFI contracts until each has its own decision.
9. The serial console goes to the Instance log, so a firmware or boot-manager failure is visible without a screen.

## Consequences

The Instance interface does not change: a caller cannot tell a UEFI Instance from a PVH one except by its Generation.
The trusted computing base grows by Cloud Hypervisor, its UEFI firmware, and a software TPM per UEFI Instance; each is pinned by digest and runs jailed.
UEFI Instances do not get snapshot-based fast start or density features until those are designed for them.
Microsoft licensing for Windows Generations is a product decision outside this repository.

## Delivery

1. This ADR, the MISSION change, and the version 1001 contract with its firmware property and Generation verification.
2. The Cloud Hypervisor machine source, its jail profile, and the per-Instance TPM.
3. The vsock session adapter and the guest-agent handshake with envd.
4. ACPI stop, `Unsupported` lifecycle answers, and the per-Instance overhead figure for capacity.
5. A Windows Generation created, stopped and started through the SOMA API on a lab host, with the Instance edge suite.
