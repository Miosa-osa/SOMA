# x86_64 KVM halt-guest proof - 2026-08-29

## Evidence boundary

This result proves that SOMA can, on a real Ubuntu 24.04 x86_64 host with `/dev/kvm`, open KVM, verify the required capability contract, create a VM, map and register 128 MiB of private guest RAM as one memory slot, write the PVH boot pages and a raw 32-bit program into that RAM, create one vCPU in 32-bit protected mode with the machine-contract register state, enter `KVM_RUN` on a dedicated thread, capture four port-I/O exits, observe `KVM_EXIT_HLT`, join the vCPU thread, and release every descriptor and mapping it opened.
It also proves that when the in-kernel interrupt controller is present, `hlt` parks the vCPU inside KVM and the watchdog interrupts and joins it after the deadline without leaking descriptors.

It does not prove Linux kernel boot, PVH kernel entry, any virtio device, a Generation, a guest agent, authenticated readiness, OCI execution, network or disk isolation, snapshot restore, or any latency objective.
The recorded timings are one-sample diagnostic numbers for an unoptimized debug build and are not a benchmark.

## Identities

- SOMA Git revision before this change: `18c8014fc1e69df7ef66f3ae09f599df87adee20`.
- Host kernel: `Linux 7.0.0-30-generic #30~24.04.1-Ubuntu SMP PREEMPT_DYNAMIC` x86_64.
- Host distribution: Ubuntu 24.04.4 LTS.
- CPU: Intel Core Ultra 9 275HX, microcode `0x11b`, `kvm_intel` loaded.
- KVM probe: `kvm-api-12-vcpu-mmap-12288` from `cargo run --locked -p soma-cli -- --backend kvm doctor` (`runtime-ready: yes`, `production-ready: no`).
- Rust toolchain: `1.98.0-x86_64-unknown-linux-gnu`.
- Guest program: the 18-byte `HALT_PROGRAM` in `crates/soma-kvm/src/x86_64/guest.rs`, loaded at guest-physical `0x01000000`.
- Guest RAM: 128 MiB anonymous `MAP_PRIVATE | MAP_NORESERVE` mapping registered as slot 0 at guest-physical 0.

## Invocation

Each ignored test ran as the only selected test in its own process because the watchdog installs a process-wide signal handler.

```sh
cargo run --locked -p soma-cli -- --backend kvm doctor
cargo test --locked -p soma-kvm --test x86_64_halt_guest \
  live::halts_after_writing_soma_to_the_serial_port_and_releases_descriptors \
  -- --ignored --exact --nocapture
cargo test --locked -p soma-kvm --test x86_64_halt_guest \
  live::in_kernel_irqchip_parks_hlt_and_the_watchdog_reclaims_the_vcpu \
  -- --ignored --exact --nocapture
```

## Measured boundary

`total_ns` starts immediately before `Kvm::new()` and stops after the VM, KVM descriptor, and guest mapping are released.
Each `phase` value is the monotonic time between the completion of the previous phase and the completion of the named phase.
The `Probe` phase includes the separate capability probe, which opens its own KVM descriptor and creates and destroys an empty VM.
The `Run` phase includes thread creation, signal-mask installation, `KVM_RUN` until `hlt`, and the join.
The descriptor counts are taken outside the timer.

## Result

The halt-guest test exited with status 0 and printed:

```text
phase=Open elapsed_ns=1838
phase=Probe elapsed_ns=19935750
phase=CreateVm elapsed_ns=1730149
phase=MapMemory elapsed_ns=4724
phase=RegisterMemory elapsed_ns=64659
phase=TssAddress elapsed_ns=2428
phase=LoadGuest elapsed_ns=20841
phase=CreateVcpu elapsed_ns=2296471
phase=Cpuid elapsed_ns=64166
phase=Regs elapsed_ns=16070
phase=Run elapsed_ns=233092
phase=Cleanup elapsed_ns=14031410
serial="SOMA" exit=Halt total_ns=38401598 fd_before=4 fd_after=4
```

The in-kernel interrupt-controller test exited with status 0 and printed:

```text
error=run vCPU 0: guest did not halt before the deadline fd_before=4 fd_after=4
```

The second test used a two-second deadline and completed in 2.04 seconds, which shows the watchdog kick interrupted a vCPU that KVM had parked in its halted state.

These are one-sample diagnostic results for a raw machine-floor tracer bullet.
They must not be compared with a kernel boot, a restored snapshot, an authenticated Ready result, or any create-through-first-command benchmark.
