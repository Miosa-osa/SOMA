# SOMA x86_64 machine contract v1

## Decision

The first SOMA KVM machine boots a pinned uncompressed x86_64 Linux ELF kernel through the PVH direct-boot ABI.
The kernel must contain the `XEN_ELFNOTE_PHYS32_ENTRY` note and must be built for the fixed SOMA machine contract.

PVH is selected because it provides a legacy-free direct entry without BIOS, UEFI, a bootloader, PCI discovery, or a general PC platform.
The cold boot path is a Generation-building and diagnostic path rather than the 10 ms request path.
The request path restores a certified snapshot of this exact machine contract.

The first executable prototype supports one boot vCPU and 128 MiB through 3 GiB of guest RAM in 4 KiB increments.
SMP and RAM above the 32-bit MMIO boundary require a later compatible machine-contract version or an explicit extension to version 1.

## Primary sources

- The [Linux x86 boot protocol](https://docs.kernel.org/arch/x86/boot.html) defines direct protected-mode and 64-bit boot state.
- The [Xen x86 HVM direct-boot ABI](https://xenbits.xen.org/docs/4.10-testing/misc/pvh.html) defines PVH entry state and the `%ebx` start-info pointer.
- The [Xen `hvm_start_info` header](https://xenbits.xen.org/docs/unstable/hypercall/x86_64/include%2Cpublic%2Carch-x86%2Chvm%2Cstart_info.h.html) defines the versioned start-info, module, command-line, and memory-map fields.
- The [Linux KVM API](https://docs.kernel.org/virt/kvm/api.html) defines VM, memory-slot, vCPU, register, interrupt, clock, and execution ioctls.
- The [rust-vmm Linux loader](https://github.com/rust-vmm/linux-loader) parses ELF load segments and the PVH entry note and writes PVH boot parameters.
- The [rust-vmm reference VMM design](https://github.com/rust-vmm/vmm-reference/blob/main/docs/DESIGN.md) records the required KVM, memory, register, interrupt, legacy-device, and kernel-loading responsibilities.
- The [Firecracker kernel setup](https://github.com/firecracker-microvm/firecracker/blob/main/docs/rootfs-and-kernel-setup.md) explains why an uncompressed x86_64 kernel avoids the decompression cost of `bzImage`.

`linux-loader` 0.14.0 is archived upstream as of 2026-08-17.
SOMA may use the pinned version for the prototype, but must not make an archived parser an unreviewed permanent dependency.
Before stable release, SOMA must either adopt a maintained successor, vendor the reviewed minimum with provenance, or implement the fixed PVH parser behind equivalent hostile-input tests.

## Guest physical layout

All addresses are guest-physical byte addresses.
Every range is checked for overflow, overlap, alignment, and containment before any KVM memory slot or guest byte is published.

| Start | End inclusive | Size | Owner | Contract |
| ---: | ---: | ---: | --- | --- |
| `0x00000000` | `0x00005fff` | 24 KiB | Reserved | Address zero remains unmapped by convention and the low area catches null or malformed boot pointers. |
| `0x00006000` | `0x00006fff` | 4 KiB | PVH start page | Contains one 56-byte `hvm_start_info` followed by zeroes. |
| `0x00007000` | `0x00007fff` | 4 KiB | PVH memory-map page | Contains bounded 24-byte `hvm_memmap_table_entry` values. |
| `0x00008000` | `0x00008fff` | 4 KiB | PVH module page | Contains at most one 32-byte initramfs module entry in version 1. |
| `0x00009000` | `0x0000afff` | 8 KiB | Kernel command line | NUL-terminated ASCII, maximum 8,191 bytes including SOMA-owned arguments. |
| `0x0000b000` | `0x0009ffff` | 596 KiB | Reserved low-memory workspace | Kept out of kernel, initramfs, and device allocations. |
| `0x000a0000` | `0x000fffff` | 384 KiB | Reserved legacy hole | Reported reserved even though SOMA implements no VGA, BIOS, or legacy ROM. |
| `0x00100000` | `0x00ffffff` | 15 MiB | Reserved loader gap | Prevents low structures and linked kernel segments from colliding. |
| ELF-linked address | ELF segment end | Variable | Pinned Linux kernel | Every `PT_LOAD` segment is loaded at its declared physical address and must be at or above `0x01000000`. |
| Top-down below guest RAM end | Guest RAM end | Variable | Initramfs | Page-aligned, non-overlapping, and represented by the sole PVH module entry. |

The pinned kernel uses `CONFIG_PHYSICAL_START=0x01000000`.
The ELF loader rejects a missing PVH note, a PVH entry outside executable loaded segments, a segment below `0x01000000`, a segment outside RAM, overlapping segments, arithmetic overflow, or collision with another reserved artifact.

Version 1 reports two PVH memory-map entries when guest RAM crosses the legacy hole.

| Address | Size | Type |
| ---: | ---: | --- |
| `0x00000000` | `0x000a0000` | RAM |
| `0x000a0000` | `0x00060000` | Reserved |
| `0x00100000` | `guest_memory_bytes - 0x00100000` | RAM |

The fixed boot pages live inside ranges described as RAM because the guest consumes them during boot.
The VMM separately prevents artifact overlap when constructing the Generation.

## PVH start information

The start page contains:

- `magic = 0x336ec578`.
- `version = 1`.
- `flags = 0`.
- `nr_modules = 0` or `1`.
- `modlist_paddr = 0` when no initramfs exists, otherwise `0x8000`.
- `cmdline_paddr = 0x9000`.
- `rsdp_paddr = 0` because version 1 exposes no ACPI tables.
- `memmap_paddr = 0x7000`.
- `memmap_entries` equal to the exact validated table length.
- Every reserved field set to zero.

The optional initramfs module entry contains its exact physical start, exact byte length, a zero module command-line pointer, and a zero reserved field.
No boot structure or initramfs address may equal zero or cross 4 GiB.

## Kernel command line

The first diagnostic command line is generated from a fixed ordered set:

```text
console=ttyS0 reboot=k panic=1 nomodule random.trust_cpu=off pci=off acpi=off noapic cryptomgr.notests
```

Production snapshots may disable the serial console after boot diagnostics are complete.
Root selection, read-only policy, init path, virtio-mmio declarations, and network bootstrap are added only by their owning Generation and device tickets.
Callers cannot inject arbitrary kernel arguments at Launch.
The complete command line is part of `GenerationId` and snapshot compatibility.

## Initial vCPU state

The PVH entry point is the 32-bit physical address carried by `XEN_ELFNOTE_PHYS32_ENTRY`.
The bootstrap vCPU enters 32-bit protected mode with paging disabled.

- `RIP` equals the validated PVH entry point.
- `RBX` equals `0x6000`, the physical address of `hvm_start_info`.
- `RFLAGS` has the architectural reserved bit set and has IF, TF, and VM cleared.
- `CR0.PE` is set and other writable CR0 bits begin cleared as required by the PVH ABI.
- `CR4` begins cleared.
- `CS` is a flat 32-bit readable and executable segment with base zero and limit `0xffffffff`.
- `DS`, `ES`, and `SS` are flat 32-bit readable and writable segments with base zero and limit `0xffffffff`.
- `TR` is a present active 32-bit TSS with base zero and limit `0x67`.
- Other general registers are zeroed by SOMA even where the ABI leaves them unspecified.
- Interrupt injection remains disabled until the in-kernel interrupt controller and event routes are complete.

The VMM obtains the supported CPUID set from KVM, applies a versioned SOMA CPU template, and installs it with `KVM_SET_CPUID2` before vCPU execution.
The exact CPUID leaves, MSR index list, LAPIC state, FPU state, XCR state, and clock state become certified snapshot fields rather than host-dependent defaults.

## Required KVM capabilities

The host profile must report KVM API version 12 and provide:

- `KVM_CAP_USER_MEMORY`.
- `KVM_CAP_IRQCHIP`.
- `KVM_CAP_IRQFD`.
- `KVM_CAP_IOEVENTFD`.
- `KVM_CAP_IMMEDIATE_EXIT`.
- A nonzero vCPU mmap size.
- At least the bounded number of memory slots required by the selected memory artifact layout.

The first cold-boot proof uses this ordering:

1. Open `/dev/kvm`.
2. Verify API version and required capabilities.
3. Create the VM with `KVM_CREATE_VM`.
4. Create and privately map guest RAM.
5. Register non-overlapping memory slots with `KVM_SET_USER_MEMORY_REGION`.
6. Configure the x86 TSS address and in-kernel interrupt controller.
7. Load and validate the kernel, start information, memory map, command line, and optional initramfs.
8. Create the bootstrap vCPU with `KVM_CREATE_VCPU`.
9. Install the filtered CPUID template.
10. Install special registers, general registers, FPU state, supported MSRs, LAPIC state, and event state.
11. Enter `KVM_RUN` on one dedicated OS thread.

Every ioctl failure is typed with its exact lifecycle phase and triggers cleanup of all previously owned resources.

## Snapshot restore ordering

Snapshot restore never replays the cold boot loader.
It restores only a Generation whose machine-contract, kernel, guest-agent, CPU-template, KVM API, host-kernel profile, and artifact digests match the certified host.

The restore order is:

1. Validate constant-size manifest identity and compatibility metadata.
2. Create the VM.
3. Privately map immutable snapshot memory without eager copying.
4. Register every memory slot at its certified guest-physical address.
5. Recreate the in-kernel interrupt controller and required device objects.
6. Create vCPU descriptors.
7. Restore CPUID and supported MSR configuration.
8. Restore special registers, general registers, FPU, XCR, XSAVE, LAPIC, MP, event, and nested state when present in the certified format.
9. Restore device queues, ioeventfd and irqfd routes, interrupt-controller state, PIT state if the selected profile contains it, and KVM clock state.
10. Attach fresh private disk, network, control, entropy, and Instance authority.
11. Resume the vCPU.
12. Require authenticated guest repair and the fixed readiness command before publishing Ready.

The snapshot format must distinguish absent optional state from silently defaulted state.
An unsupported ioctl, CPUID feature, MSR, state size, device version, or clock mode rejects restore before vCPU execution.

## Failure and cleanup contract

Failure before `KVM_RUN` closes vCPU, VM, KVM, memory, device, event, disk, network, and control resources in reverse ownership order.
Failure after `KVM_RUN` first requests immediate vCPU exit, joins the vCPU thread within a fixed deadline, then performs the same cleanup.
An unjoined vCPU or unproven resource release produces incomplete cleanup evidence and prevents process reuse.

No failure path starts a host process, Docker container, Apple VM, or another weaker backend.
The KVM request either satisfies this machine contract or fails closed.

## Explicitly unsupported in version 1

- BIOS and UEFI.
- A bootloader inside the guest.
- Real-mode Linux boot.
- `bzImage` on the certified fast profile.
- ACPI and SMBIOS.
- PCI and PCIe.
- VGA, framebuffer, graphics, keyboard, mouse, USB, audio, and generic PC chipset emulation.
- Device, CPU, or memory hotplug.
- Guest-supplied kernels or kernel command lines at Launch.
- Nested virtualization.
- Confidential-computing modes.
- Live migration.
- Cross-architecture execution.
- More than one initramfs module.

Later machine-contract versions may add capabilities only with new compatibility evidence and without changing the meaning of version 1 snapshots.

## Linux implementation acceptance

Ticket #4 is implemented only when a Linux x86_64 integration test:

1. Builds or verifies the pinned kernel and initramfs fixture.
2. Creates one KVM VM with the exact layout above.
3. Enters the validated PVH entry point.
4. Captures a challenge-bound guest signal over the diagnostic console.
5. Terminates the vCPU and proves descriptor and memory cleanup.
6. Retains the host, kernel, KVM, artifact, command-line, and timing evidence.

That proof is a cold-boot machine-contract test.
It is not a working OCI sandbox, authenticated Ready result, snapshot restore, or 10 ms performance claim.
