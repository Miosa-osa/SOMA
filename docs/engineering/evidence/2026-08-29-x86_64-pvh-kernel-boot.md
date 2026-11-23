# x86_64 PVH kernel cold-boot proof - 2026-08-29

## Evidence boundary

This result proves that SOMA can, on a real Ubuntu 24.04 x86_64 host with `/dev/kvm`, parse the pinned uncompressed Linux ELF kernel with its own bounded PVH parser, load its segments and a top-down initramfs into private guest RAM, write the PVH start page, memory map, module entry, and command line at the machine-contract addresses, create the in-kernel interrupt controller and programmable interval timer, enter the validated PVH entry on one 32-bit protected-mode vCPU, answer the kernel's port I/O through a bounded 16550 model and an irqfd-backed transmit interrupt, receive a challenge-bound sentinel written by a static `/init` through the Linux 8250 driver, observe the `reboot=k` keyboard-controller reset pulse as an orderly exit, join the vCPU thread, and release every descriptor and mapping it opened.
It is the machine-contract acceptance test named in [the x86_64 machine contract](../research/x86_64-machine-contract.md).

It does not prove any virtio device, an MMIO bus, a root filesystem, a Generation, a guest agent, authenticated readiness, OCI execution, network or disk isolation, snapshot restore, or any latency objective.
The recorded timings are single-sample diagnostic numbers for an unoptimized debug build and are not a benchmark.
The host-residency numbers are single-sample diagnostics and are not a certified per-VM overhead figure.

## Identities

- SOMA Git revision before this change: `16544c6` (`origin/main` at the time of the rebase); the branch base before the rebase was `388ac46`.
- Host kernel: `Linux 7.0.0-30-generic #30~24.04.1-Ubuntu SMP PREEMPT_DYNAMIC` x86_64.
- Host distribution: Ubuntu 24.04.4 LTS.
- CPU: Intel Core Ultra 9 275HX, microcode `0x11b`, `kvm_intel` loaded.
- KVM probe: `cargo run --locked -p soma-cli -- --backend kvm doctor` reported `doctor: probe passed`, `runtime-ready: yes`, `production-ready: no`.
- Rust toolchain: `1.98.0 (88d9e12ae 2026-08-18)`.
- Guest kernel: `kernel/out/vmlinux-6.12.107-soma-v1`, Linux `6.12.107-soma-v1`, 21,530,432 bytes, SHA-256 `f1af3a142fa39916cfac425a01b16b5f328279823533421c9eec3f192c05b746`.
- That digest is the `CONFIG_DEVMEM=y` rebuild recorded in `kernel/out/manifest.json`; the earlier `CONFIG_DEVMEM=n` build `cf071d83d5461a0b739a5c361825f994a528e0b5bee1b9b78350e5f07b22755c` also boots with this slice but is not the retained result.
- The kernel's `XEN_ELFNOTE_PHYS32_ENTRY` note carries an eight-byte descriptor equal to `0x01000000`, and its four `PT_LOAD` segments start at `0x01000000`, `0x01a00000`, `0x01d22000`, and `0x01d47000`.
- Init fixture: `crates/soma-kvm/tests/fixtures/x86_64/x86_64_init.c` compiled by `build_x86_64_init.py` inside the pinned `soma-kernel-builder:local` image (gcc 13.3.0), 8,944 bytes, packed at test time by the test-local `newc` writer into a 9,540-byte initramfs containing `/init`, `/dev/console` (character 5:1), and `/proc`.
- Guest RAM: 256 MiB anonymous `MAP_PRIVATE | MAP_NORESERVE` mapping registered as slot 0 at guest-physical 0.
- Initramfs placement: guest-physical `0x0fffd000`, 9,540 bytes, reported as the sole PVH module.

## Command line

The command line is composed in one place, `crates/soma-kvm/src/x86_64/cmdline.rs`, from the fixed ordered contract set plus the two arguments the fixture needs:

```text
console=ttyS0 reboot=k panic=1 nomodule random.trust_cpu=off pci=off acpi=off noapic cryptomgr.notests rdinit=/init soma.nonce=b31773bed228c735
```

The nonce is eight fresh bytes from `/dev/urandom` per run, and the guest must write `SOMA-BOOT-<nonce>` on its own console line.

## Invocation

```sh
SOMA_X86_64_VMLINUX=kernel/out/vmlinux-6.12.107-soma-v1 \
  cargo test --locked -p soma-kvm --test x86_64_kernel_boot -- --ignored --test-threads=1 --nocapture
cargo test --locked -p soma-kvm --test x86_64_halt_guest -- --ignored --test-threads=1
```

Without `SOMA_X86_64_VMLINUX` the test polls `kernel/out/` and a sibling `pinned-x86-64-kernel/kernel/out/` checkout for a size-stable `vmlinux-<version>-soma-v1` whose digest appears in `manifest.json`, for up to 45 minutes, and fails with the searched paths if none appears.

## Measured boundary

`total_ns` starts before the kernel file is read and stops after the VM, KVM descriptor, and guest mapping are released.
Each `phase` value is the monotonic time between the completion of the previous phase and the completion of the named phase.
`ReadKernel` reads the 21.5 MiB kernel and the initramfs into memory.
`Run` covers thread creation, signal-mask installation, `KVM_RUN` from the PVH entry until the reset pulse, and the join.
The descriptor counts are taken outside the timer.

## Result

The acceptance test passed on every run; the retained run printed:

```text
phase=ReadKernel elapsed_ns=8013727
phase=Open elapsed_ns=8122
phase=Probe elapsed_ns=11218173
phase=CreateVm elapsed_ns=703992
phase=MapMemory elapsed_ns=7349
phase=RegisterMemory elapsed_ns=163256
phase=TssAddress elapsed_ns=7678
phase=IrqChip elapsed_ns=37570
phase=Pit elapsed_ns=39565
phase=LoadGuest elapsed_ns=6408425
phase=CreateVcpu elapsed_ns=345428
phase=Cpuid elapsed_ns=72992
phase=Regs elapsed_ns=11722
phase=Run elapsed_ns=144374768
phase=Cleanup elapsed_ns=23805911
exit=Reset total_ns=195218678 fd_before=4 fd_after=4
bus=BusCounters { serial_in: 10592, serial_out: 10999, i8042_in: 1, i8042_out: 1, other_in: 0, other_out: 0 }
uart=SerialCounters { thr_writes: 10652, ier_writes: 318, lsr_reads: 10412, iir_reads: 16, other_reads: 3, other_writes: 0, interrupts_raised: 34 }
```

The `Run` phase, which is the time from `KVM_RUN` entry through the sentinel and the guest's own reboot, was 115 ms, 118 ms, 127 ms, 144 ms, 153 ms, 155 ms, 167 ms, and 586 ms across eight runs on a busy development host; the guest's own `printk` clock reached `/init` between 0.10 s and 0.49 s.
These are debug-build, single-vCPU, cold-boot observations under host contention and must not be compared with a restored snapshot or any Ready or first-command result.

The guest touched no port outside the serial model and the two keyboard-controller accesses of the reboot path, so `other_in` and `other_out` are zero.

The negative test corrupted the `Xen` owner name of the PVH note in a copy of the kernel and printed:

```text
error=load guest program and boot structures: kernel ELF rejected: no XEN_ELFNOTE_PHYS32_ENTRY note is present fd_before=4 fd_after=4
```

The rejection occurred in the `LoadGuest` phase before any vCPU was created.

The halt-guest floor still passed both of its live tests after this change.

## Serial excerpt

The full 152-line, 10,651-byte log is retained by the test under `target/tmp/x86_64-kernel-boot/serial.log`.
The first lines are:

```text
[    0.000000] Linux version 6.12.107-soma-v1 (soma@soma-kernel-builder) (gcc (Ubuntu 13.3.0-6ubuntu2~24.04.1) 13.3.0, GNU ld (GNU Binutils for Ubuntu) 2.42) #1 SMP PREEMPT_DYNAMIC 2026-08-29T00:00:00Z
[    0.000000] Command line: console=ttyS0 reboot=k panic=1 nomodule random.trust_cpu=off pci=off acpi=off noapic cryptomgr.notests rdinit=/init soma.nonce=188e6a4ce08bf333
[    0.000000] Disabled fast string operations
[    0.000000] x86/split lock detection: #DB: warning on user-space bus_locks
[    0.000000] BIOS-provided physical RAM map:
[    0.000000] BIOS-e820: [mem 0x0000000000000000-0x000000000009ffff] usable
[    0.000000] BIOS-e820: [mem 0x00000000000a0000-0x00000000000fffff] reserved
[    0.000000] BIOS-e820: [mem 0x0000000000100000-0x000000000fffffff] usable
[    0.000000] NX (Execute Disable) protection: active
[    0.000000] APIC: Static calls initialized
[    0.000000] Hypervisor detected: KVM
[    0.000000] last_pfn = 0x10000 max_arch_pfn = 0x400000000
[    0.000000] kvm-clock: Using msrs 4b564d01 and 4b564d00
[    0.000000] kvm-clock: using sched offset of 12039886 cycles
[    0.000004] clocksource: kvm-clock: mask: 0xffffffffffffffff max_cycles: 0x1cd42e4dffb, max_idle_ns: 881590591483 ns
[    0.000013] tsc: Detected 3072.000 MHz processor
[    0.000103] last_pfn = 0x10000 max_arch_pfn = 0x400000000
[    0.000129] MTRRs disabled by BIOS
[    0.000134] x86/PAT: Configuration [0-7]: WB  WC  UC- UC  WB  WP  UC- WT
[    0.004248] Using GB pages for direct mapping
[    0.004322] RAMDISK: [mem 0x0fffd000-0x0fffffff]
[    0.004359] Zone ranges:
[    0.004362]   DMA      [mem 0x0000000000001000-0x0000000000ffffff]
[    0.004363]   DMA32    [mem 0x0000000001000000-0x000000000fffffff]
[    0.004364]   Normal   empty
[    0.004365] Movable zone start for each node
[    0.004367] Early memory node ranges
[    0.004367]   node   0: [mem 0x0000000000001000-0x000000000009ffff]
[    0.004369]   node   0: [mem 0x0000000000100000-0x000000000fffffff]
[    0.004370] Initmem setup node 0 [mem 0x0000000000001000-0x000000000fffffff]
```

Selected middle lines show the interrupt mode and console detection:

```text
[    0.011159] printk: legacy console [ttyS0] enabled
[    0.047643] APIC: ACPI MADT or MP tables are not detected
[    0.048047] APIC: Switch to virtual wire mode setup with no configuration
[    0.079193] clocksource: Switched to clocksource kvm-clock
[    0.085296] Freeing initrd memory: 12K
[    0.094327] Serial: 8250/16550 driver, 1 ports, IRQ sharing disabled
[    0.094884] serial8250: ttyS0 at I/O 0x3f8 (irq = 4, base_baud = 115200) is a 16550A
```

The last lines are:

```text
[    0.099947] printk: legacy console [netcon0] enabled
[    0.100307] netconsole: network logging started
[    0.101200] Freeing unused kernel image (initmem) memory: 1368K
[    0.101456] Write protecting the kernel read-only data: 10240k
[    0.101828] Freeing unused kernel image (rodata/data gap) memory: 176K
[    0.102204] x86/mm: Checked W+X mappings: passed, no W+X pages found.
[    0.102458] Run /init as init process
SOMA-BOOT-188e6a4ce08bf333
[    0.123236] reboot: Restarting system
[    0.123401] reboot: machine restart
```

The e820 map is exactly the three-entry contract map, the guest selected `kvm-clock`, and the 8250 driver detected `ttyS0` as a `16550A` on IRQ 4.

## Host-side resident overhead for one 1-vCPU guest with 256 MiB RAM

Debug build, single sample per line, taken by a 2 ms sampler thread inside the test process from `/proc/self/status`, `/proc/self/smaps_rollup`, `/proc/self/smaps`, and `/proc/self/fd`.
The guest RAM mapping itself is anonymous `MAP_PRIVATE`, so its resident pages are part of `RssAnon`; the `guest_mapping_rss` value isolates that mapping.
This is a single-sample diagnostic and not a certified overhead figure.

| Sample | VmRSS | RssAnon | RssFile | RssShmem | Guest mapping Rss | Threads | Open fds |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Test process before the run | 3,284 kB | 288 kB | 2,996 kB | 0 kB | none | 2 | 4 |
| Last sample with the guest mapped | 51,300 kB | 48,184 kB | 3,116 kB | 0 kB | 26,684 kB | 3 | 5 |
| Peak `VmRSS` while running | 51,304 kB | 48,184 kB | 3,116 kB | 4 kB | 26,684 kB | 3 | 5 |
| Maximum seen at any poll | - | - | - | - | - | 5 | 8 |

The `smaps_rollup` line at the peak was `Rss: 51300 kB, Pss: 49482 kB, Private_Dirty: 48184 kB, Anonymous: 48184 kB`.

Reading the anonymous total: 26,684 kB is guest RAM the kernel actually touched out of 262,144 kB registered, about 21,028 kB is the kernel image buffer the loader holds in host memory for the duration of the run (21,530,432 bytes, an implementation detail that a production loader would map or stream instead), and the remaining roughly 470 kB is the test process, KVM run page, and allocator overhead.
Excluding the kernel image buffer and the guest-touched pages, the host-side anonymous overhead of this VMM process was below 1 MiB, and the non-guest resident total including file-backed code was about 3.6 MiB.
The maximum of 8 open descriptors is the baseline 4 plus the KVM, VM, vCPU, and serial-interrupt eventfd descriptors; the maximum of 5 threads is the test harness, the test, the vCPU thread, the sampler, and one transient thread the sampler observed once.
The `visual-atlas.md` capacity placeholder of 64 MiB per VM should not be replaced by these numbers without a release-build, multi-sample measurement of the real `soma-vmm` process with its devices attached.

## Diagnostic observations

- Without `KVM_CREATE_PIT2`, the kernel still booted to `/init` and wrote the sentinel, but `/init`'s 20 ms `nanosleep` never returned and the run ended in the watchdog timeout; the local APIC timer is calibrated against the PIT, so the PIT is a required element of the version 1 profile rather than an optional one.
- The tty layer converts the fixture's `\n` to `\r\n`; the test therefore matches the sentinel as a complete line rather than as a raw byte string.
- The Linux 8250 driver enabled the transmit-holding-register interrupt 34 times during boot and the irqfd delivered each one through the in-kernel PIC on IRQ 4; without that interrupt tty writes longer than the FIFO depth would stall.
- `netconsole` is compiled into the pinned kernel and registers `netcon0` with no network device; it is harmless here but is a candidate for removal from the kernel configuration.

## What this does not prove

- No virtio-mmio bus dispatch, transport, block, network, vsock, or entropy device was exercised; the guest saw no device other than the UART.
- No root filesystem, EROFS image, or overlay was mounted; the guest ran entirely from a 9.5 KiB initramfs.
- No guest agent, authenticated repair, readiness probe, command execution, or `Ready` result exists.
- No snapshot capture or restore occurred.
- No isolation, seccomp, namespace, cgroup, or network policy was applied to the VMM process.
- No latency claim: the numbers are unoptimized debug-build cold-boot observations under host contention.
