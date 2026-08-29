# SOMA x86_64 guest kernel (machine contract v1)

## Purpose

This directory produces the pinned guest kernel required by the [x86_64 machine contract v1](../docs/research/x86_64-machine-contract.md) and the [Generation compiler kernel contract](../docs/research/generation-compiler.md).
The output is one uncompressed Linux ELF `vmlinux` that carries the `XEN_ELFNOTE_PHYS32_ENTRY` note, links every `PT_LOAD` segment at or above guest-physical `0x01000000`, and contains only the facilities the [minimal device surface](../docs/research/minimal-device-surface.md) needs.
The build is reproducible: the same inputs produce the same `vmlinux` bytes, and every input is pinned by digest.

The kernel is a Generation input.
It is not a running sandbox, a boot proof, or a performance result.

## Pinned inputs

| Input | Pin |
| --- | --- |
| Linux source | `v6.12.107` from kernel.org, `linux-6.12.107.tar.xz`, SHA-256 `a5f8c5be3fde2d6d9ca14e9631642cf1f44487143f11059da730dcd5892e307a` |
| Configuration | `config-x86_64-soma-v1` (full `.config`, regenerated from `soma-v1.fragment` on top of `x86_64_defconfig`) |
| Contract symbols | `required-config.txt` (the machine-contract symbols that must hold after `make olddefconfig`) |
| Builder base image | `ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517` (the `docker.io/library/ubuntu:24.04` index digest on 2026-08-29) |
| Toolchain | gcc `13.3.0` (`gcc-13 13.3.0-6ubuntu2~24.04.1`), GNU ld `2.42` (`binutils 2.42-4ubuntu2.10`), GNU Make `4.3` (`make 4.3-4.1build2`) |
| Build metadata | `KBUILD_BUILD_TIMESTAMP=2026-08-29T00:00:00Z`, `KBUILD_BUILD_USER=soma`, `KBUILD_BUILD_HOST=soma-kernel-builder`, `KBUILD_BUILD_VERSION=1`, `SOURCE_DATE_EPOCH=1756425600` |

All pins live in `source.json`, which `build.sh` reads and enforces.

Linux 6.12 is the current longest-supported LTS line that predates the project's verification date and is the same major line Firecracker validates its own x86_64 guest configurations against, while 6.6 would give up two years of virtio, EROFS, and OverlayFS fixes for no contract benefit.
`6.12.107` was the newest 6.12.y tarball published on kernel.org on 2026-08-29.

The toolchain pin is a base-image digest plus verified package versions rather than an `apt` snapshot.
Ubuntu's `snapshot.ubuntu.com` service did not honour the snapshot request from this image during setup, so `build.sh` instead fails closed when `gcc -dumpfullversion`, `ld --version`, or `make --version` differ from `source.json`.
If Ubuntu publishes a newer gcc-13 or binutils build the pinned image must be re-reviewed and the kernel rebuilt as a new Generation input, not silently accepted.

## Build

Requirements: Linux x86_64 host, Docker without root, `curl`, `python3`, `sha256sum`.
No kernel build dependencies are needed on the host; everything compiles inside the builder container.

```sh
kernel/build.sh
```

The script performs, in order:

1. Downloads `linux-6.12.107.tar.xz` into `kernel/out/src/` unless a tarball with the pinned SHA-256 already exists, and refuses any other digest.
2. Builds the builder image from `Dockerfile` and records its image identity.
3. Runs the container with `--network none`, the invoking user's uid and gid, and the fixed `KBUILD_*` and `SOURCE_DATE_EPOCH` values.
4. Inside the container verifies gcc, ld, and make versions, extracts a fresh tree, copies `config-x86_64-soma-v1` to `.config`, and runs `make olddefconfig`.
5. Runs `verify-config.py`, which fails if `olddefconfig` flipped or dropped any pinned symbol or if any `required-config.txt` symbol does not hold.
6. Runs `make -j$(nproc) vmlinux` (override the job count with `SOMA_KERNEL_JOBS`).
7. Copies the result to `kernel/out/vmlinux-6.12.107-soma-v1`, writes its `.sha256`, runs `verify-pvh.py`, and writes `kernel/out/manifest.json`.

`kernel/build.sh regen-config` regenerates `config-x86_64-soma-v1` from `soma-v1.fragment` and must be followed by a review of the resulting diff.

## Outputs

| Path | Content |
| --- | --- |
| `kernel/out/vmlinux-6.12.107-soma-v1` | Uncompressed ELF64 x86_64 kernel with the PVH note; the 2026-08-29 reproducible digest is `cf071d83d5461a0b739a5c361825f994a528e0b5bee1b9b78350e5f07b22755c` (21,530,056 bytes) |
| `kernel/out/vmlinux-6.12.107-soma-v1.sha256` | SHA-256 of the kernel |
| `kernel/out/manifest.json` | Source tag and digest, config digests, builder image identity and package versions, build environment, job count, timings, output digest and size, and the PVH verification report |
| `kernel/out/final.config` | The `.config` after `make olddefconfig`, which must be identical to the pinned config for every pinned symbol |
| `kernel/out/build.log` | Complete container output |

`kernel/out/` is ignored by Git.
Binaries are never committed; the manifest digests are what a Generation records.

## PVH verification

`verify-pvh.py` uses only the Python standard library and fails closed.
It confirms that the file is ELF64, little-endian, `ET_EXEC`, and `EM_X86_64`; that every `PT_LOAD` segment starts at or above `0x01000000`, ends below 4 GiB, and overlaps no other segment; that exactly one `PT_NOTE` note has name `Xen` and type `18` (`XEN_ELFNOTE_PHYS32_ENTRY`); that the note's descriptor is a 4- or 8-byte little-endian value that fits in 32 bits; and that the entry lies inside the file-backed part of an executable `PT_LOAD` segment.

```sh
python3 kernel/verify-pvh.py kernel/out/vmlinux-6.12.107-soma-v1
python3 kernel/verify-pvh.py kernel/out/vmlinux-6.12.107-soma-v1 --json
```

The KVM boot slice must run the same checks in Rust before loading a segment, because the Python verifier is build evidence and not the runtime loader.

## Configuration notes

The full config is the artifact; `soma-v1.fragment` documents every intended delta from `x86_64_defconfig`.

- `CONFIG_PVH=y`, `CONFIG_KVM_GUEST=y`, `CONFIG_PARAVIRT=y`, `CONFIG_PHYSICAL_START=0x1000000`, `CONFIG_RELOCATABLE=n`, and `CONFIG_RANDOMIZE_BASE=n` fix the link address required by the machine contract.
- `CONFIG_ACPI=n`, `CONFIG_PCI=n`, `CONFIG_EFI=n`, `CONFIG_XEN=n`, `CONFIG_PNP=n`, and `CONFIG_DMI=n` remove firmware and bus discovery.
- `CONFIG_VIRTIO_MMIO=y` with `CONFIG_VIRTIO_MMIO_CMDLINE_DEVICES=y` is the only transport; `virtio_pci`, balloon, console, input, memory, filesystem, and IOMMU virtio devices are off.
- `CONFIG_VIRTIO_BLK`, `CONFIG_VIRTIO_NET`, `CONFIG_VIRTIO_VSOCKETS`, and `CONFIG_HW_RANDOM_VIRTIO` are built in.
- `CONFIG_EROFS_FS=y` without compression support (the Generation profile uses uncompressed EROFS), `CONFIG_EXT4_FS=y`, `CONFIG_OVERLAY_FS=y`, `devtmpfs` with automount, `procfs`, `sysfs`, and `tmpfs`.
- `CONFIG_MODULES=n`, so the guest cannot load code the Generation did not certify.
- `CONFIG_NET=y` with IPv4 and IPv6, `CONFIG_NETFILTER=n` because the host profile enforces policy, and no Ethernet, wireless, or Bluetooth drivers.
- `CONFIG_SERIAL_8250=y` with one UART and console support for the diagnostic `ttyS0`; `CONFIG_VT=n` and `CONFIG_INPUT=n`.
- `CONFIG_SECCOMP_FILTER=y`, namespaces, and a minimal cgroup set (`pids`, `memory`, `cpu`, `cpuset`).
- No USB, sound, DRM, framebuffer, SCSI, ATA, MD, device mapper, loop, NVMe, RTC, HPET device, watchdog, hotplug memory, or hardware monitoring.
- `CONFIG_DEBUG_INFO_NONE=y`, no BTF, no ftrace, no kprobes, no profiling, no KUnit, no runtime tests.
- `CONFIG_IKCONFIG_PROC=y` exposes the exact configuration at `/proc/config.gz` for later Generation verification.

Symbols the fragment cannot disable in Linux 6.12 because Kconfig forces them: `CONFIG_MICROCODE` (`def_bool y`), `CONFIG_HOTPLUG_CPU` (`def_bool y` when `SMP`), `CONFIG_HPET_TIMER` (`def_bool X86_64`), `CONFIG_PERF_EVENTS` (selected by `X86`), `CONFIG_DEBUG_KERNEL` (selected by `EXPERT`, gates only the debug menu), and `CONFIG_NET_FAILOVER` (selected by `VIRTIO_NET`).
`CONFIG_RANDOM_TRUST_CPU` no longer exists in 6.12; the kernel trusts the CPU by default and the machine-contract command line disables that with `random.trust_cpu=off`.
`CONFIG_SMP=y` remains so a later machine-contract version can add vCPUs without a different kernel; version 1 boots one vCPU and the SMP kernel does not require an MP table for that.

## Pending change: `/dev/mem` for the guest agent

The built kernel has `CONFIG_DEVMEM=n`, but the guest agent must read the SOMA launch page through `/dev/mem`.
`soma-v1.fragment` now requests `CONFIG_DEVMEM=y` with `CONFIG_STRICT_DEVMEM=n`, while `config-x86_64-soma-v1`, the manifest, and the retained evidence still describe the `CONFIG_DEVMEM=n` build.
The next step is `kernel/build.sh regen-config`, review of the config diff, a rebuild, and a new evidence document; until then the recorded digest `cf071d83d5461a0b739a5c361825f994a528e0b5bee1b9b78350e5f07b22755c` is the only built artifact and it cannot serve the launch-page contract.

## What this does not claim

- No boot evidence exists yet.
  The kernel has not been entered through KVM, and the serial console, virtio-mmio discovery, EROFS root, OverlayFS composition, vsock, and entropy paths are unverified until the KVM boot slice runs.
- `verify-pvh.py` proves ELF layout and note presence, not that the PVH entry code executes correctly under SOMA's initial vCPU state.
- The toolchain is pinned by base-image digest and verified package versions, not by an immutable apt snapshot, so a future rebuild can fail closed if Ubuntu updates gcc-13 or binutils.
- Reproducibility evidence covers two builds on one host on one date and is recorded in `docs/evidence/`; it is not a cross-host reproducibility claim.
- No size, boot-time, or restore-time target is claimed.
