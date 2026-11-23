# x86_64 PVH guest kernel reproducible build - 2026-08-29

## Evidence boundary

This result proves that SOMA revision `18c8014fc1e69df7ef66f3ae09f599df87adee20` plus the `kernel/` pipeline on branch `feat/pinned-x86-64-kernel` (kernel pipeline commit `07e7fb1` and the uncommitted DEVMEM revision described below) builds Linux `v6.12.107` into an uncompressed x86_64 ELF `vmlinux` that carries `XEN_ELFNOTE_PHYS32_ENTRY`, links every `PT_LOAD` segment at or above `0x01000000`, and reproduces byte for byte across two consecutive builds on one host.
It does not prove that the kernel boots under KVM, that the PVH entry executes under SOMA's initial vCPU state, that any virtio-mmio device is discovered, that EROFS or OverlayFS mount, or any latency objective.
It is a Generation-input build proof and must not be presented as boot, sandbox, or performance evidence.

## Identities

- SOMA Git revision: `18c8014fc1e69df7ef66f3ae09f599df87adee20` (worktree branch `feat/pinned-x86-64-kernel`; the kernel pipeline was committed as `07e7fb1` and revised for `CONFIG_DEVMEM` before the recorded builds).
- Host: Intel Core Ultra 9 275HX, 24 logical CPUs, 62 GiB RAM, Ubuntu 24.04.4 LTS, Linux `7.0.0-30-generic` x86_64.
- Docker Engine: `29.3.0`, rootless invocation as the build user.
- Linux source: `linux-6.12.107.tar.xz` from `https://cdn.kernel.org/pub/linux/kernel/v6.x/`, SHA-256 `a5f8c5be3fde2d6d9ca14e9631642cf1f44487143f11059da730dcd5892e307a`, matched against the kernel.org `sha256sums.asc` listing on 2026-08-29.
- Builder base image: `ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517` (`docker.io/library/ubuntu:24.04` on 2026-08-29).
- Builder packages: `gcc-13 13.3.0-6ubuntu2~24.04.1`, `binutils 2.42-4ubuntu2.10`, `make 4.3-4.1build2`, `flex 2.6.4-8.2build1`, `bison 2:3.8.2+dfsg-1build2`, `libelf-dev 0.190-1.1ubuntu0.1`, `libssl-dev 3.0.13-0ubuntu3.15`.
- Pinned config `kernel/config-x86_64-soma-v1` SHA-256: `5369fdac4e4d691a0e21da5d99355c7006d55ae1ec8e9f3f439581d92771b28a`.
- Config fragment `kernel/soma-v1.fragment` SHA-256: `f30f2fb3f87ab6d4c37c7384db941bd8bb0d6b323d896df6804dcafc4c75d83e`.
- Required-symbol list `kernel/required-config.txt` SHA-256: `41e7e2ab3706122d57ce543e39e5becb5437232980bb21d4a03831a7af43e14c`.
- Builder `kernel/Dockerfile` SHA-256: `737d8d013c7b444157d0a20b0029477ae20d8b71b433a64c6df3851eab62fee7`.
- Build environment: `KBUILD_BUILD_TIMESTAMP=2026-08-29T00:00:00Z`, `KBUILD_BUILD_USER=soma`, `KBUILD_BUILD_HOST=soma-kernel-builder`, `KBUILD_BUILD_VERSION=1`, `SOURCE_DATE_EPOCH=1756425600`, `--network none`, 24 jobs.

## Invocation

Each build ran the same command from the worktree root.

```sh
kernel/build.sh
```

The script downloaded and verified the tarball once, built the builder image from `kernel/Dockerfile`, ran `make olddefconfig` on the pinned config, ran `kernel/verify-config.py`, ran `make -j24 vmlinux`, ran `kernel/verify-pvh.py`, and wrote `kernel/out/manifest.json`.

## Results

| Build | `make olddefconfig` drift | `make vmlinux` wall | Container wall | Total wall | vmlinux SHA-256 | Size |
| --- | --- | ---: | ---: | ---: | --- | ---: |
| 1 | none (1613 pinned symbols unchanged, 62 required symbols hold) | 49.9 s | 60 s | 60.84 s | `f1af3a142fa39916cfac425a01b16b5f328279823533421c9eec3f192c05b746` | 21,530,432 bytes |
| 2 | none (1613 pinned symbols unchanged, 62 required symbols hold) | 52.6 s | 63 s | 63.57 s | `f1af3a142fa39916cfac425a01b16b5f328279823533421c9eec3f192c05b746` | 21,530,432 bytes |

`cmp` reported the two `vmlinux` files identical.
Both post-`olddefconfig` `.config` files had SHA-256 `5369fdac4e4d691a0e21da5d99355c7006d55ae1ec8e9f3f439581d92771b28a`, identical to the pinned config.
The two manifests differed in exactly one field, `builder.image_id` (`sha256:46c37104...` versus `sha256:6fcd10bc...`), because `docker build` re-executed the `apt-get` layer and produced a new image identity each time while installing identical package versions.
The output bytes did not depend on that identity.

Loaded footprint reported by `size`: text 11,038,083 bytes, data 3,832,472 bytes, bss 1,802,244 bytes, total 16,672,799 bytes.
The 21.5 MB file size includes the symbol and string tables that are not part of any `PT_LOAD` segment.

## PVH verification

`python3 kernel/verify-pvh.py kernel/out/vmlinux-6.12.107-soma-v1` printed the following for both builds.

```text
verify-pvh: OK
  ELF64 ET_EXEC EM_X86_64, e_entry=0x1000123
  PT_LOAD paddr=0x1000000 vaddr=0xffffffff81000000 filesz=10301788 memsz=10301788 R-E
  PT_LOAD paddr=0x1a00000 vaddr=0xffffffff81a00000 filesz=3284992 memsz=3284992 RW-
  PT_LOAD paddr=0x1d22000 vaddr=0x0 filesz=149400 memsz=149400 RW-
  PT_LOAD paddr=0x1d47000 vaddr=0xffffffff81d47000 filesz=1273856 memsz=2945024 R-E
  XEN_ELFNOTE_PHYS32_ENTRY = 0x1000000 (inside executable PT_LOAD at 0x1000000)
```

`nm` confirms `pvh_start_xen` at `0xffffffff81000000`, the first byte of `_text`, so the 32-bit entry `0x01000000` equals `CONFIG_PHYSICAL_START` and `e_entry` `0x1000123` is `startup_64`.
The highest loaded byte is `0x1d47000 + 2945024 = 0x2016000`, so the kernel occupies guest-physical `0x01000000` through `0x02015fff` (about 16.1 MiB) and stays inside the 128 MiB minimum RAM of machine contract v1.
`readelf -n` also shows a `Xen` note of type 19 (`XEN_ELFNOTE_PHYS32_RELOC`, alignment `0x200000`, range `0x1000000` to `0x1fffffff`) that Linux 6.12 emits even with `CONFIG_RELOCATABLE=n`; SOMA ignores it and loads at the linked addresses.
The third `PT_LOAD` at `paddr 0x1d22000` with `vaddr 0` is the per-CPU data segment produced by `CONFIG_SMP=y`; its physical placement is contiguous and non-overlapping.

Fail-closed checks on malformed inputs each returned exit status 1: a non-ELF file (`short read while reading ELF identification`), the first 100,000 bytes of the kernel (`short read while reading PT_NOTE`), a copy with the PHYS32_ENTRY note type rewritten to 99 (`no Xen note of type XEN_ELFNOTE_PHYS32_ENTRY (18)`), and a final config with `CONFIG_PVH` flipped to `n` (`verify-config: FAIL (2 mismatches)`).

## Configuration deltas that mattered

- `CONFIG_EROFS_FS` is nested under `CONFIG_MISC_FILESYSTEMS`; disabling the miscellaneous filesystem menu silently dropped EROFS in the first fragment draft, which the required-symbol check caught.
- `CONFIG_SCHED_MC_PRIO` selects `CONFIG_CPU_FREQ`; it had to be disabled explicitly to remove cpufreq.
- `CONFIG_MICROCODE`, `CONFIG_HOTPLUG_CPU`, `CONFIG_HPET_TIMER`, `CONFIG_PERF_EVENTS`, `CONFIG_DEBUG_KERNEL`, and `CONFIG_NET_FAILOVER` cannot be disabled in 6.12 because Kconfig forces them; the fragment records why.
- `CONFIG_RANDOM_TRUST_CPU` no longer exists in 6.12, so the contract command line's `random.trust_cpu=off` is the only control.
- `CONFIG_DEVMEM=y` with `CONFIG_STRICT_DEVMEM=n` and `CONFIG_IO_STRICT_DEVMEM=n` gives the guest agent `/dev/mem` for the launch page; `make olddefconfig` consequently dropped `CONFIG_EXCLUSIVE_SYSTEM_RAM` (`def_bool y` only when `!DEVMEM || STRICT_DEVMEM`) and `CONFIG_PAGE_TABLE_CHECK` (depends on `EXCLUSIVE_SYSTEM_RAM`), and nothing else changed between the two pinned configs.

## Superseded build

Earlier the same day the pipeline built the same source with `CONFIG_DEVMEM=n` (pinned config SHA-256 `39f38021c69bfce9963926bb19b5c30cbd437d3902ab31959054af5153c8fd74`).
Two runs of that configuration also reproduced byte for byte with vmlinux SHA-256 `cf071d83d5461a0b739a5c361825f994a528e0b5bee1b9b78350e5f07b22755c`, 21,530,056 bytes, in 48.8 s and 48.1 s of `make vmlinux` time.
That artifact is superseded because the guest agent reads the SOMA launch page through `/dev/mem`, which the `CONFIG_DEVMEM=n` kernel does not provide.
It must not be used as a Generation input and is recorded here only so the digest history is complete.

## Measurement warning

Build times are wall-clock durations on one 24-core development host with a warm page cache after the first source extraction and are not build-farm numbers.
The reproducibility result covers two builds on the same host, the same day, and the same Docker daemon; cross-host reproducibility, a rebuilt base image, or a later Ubuntu toolchain release are untested.
