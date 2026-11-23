# ARM64 nested KVM cold-boot proof - 2026-08-28

## Evidence boundary

This result proves that SOMA revision `ca08d420b01e0840458220579de75682ea30f120` can direct-boot the reviewed Linux ARM64 fixture through real nested KVM, observe its exact serial sentinel, join the sole vCPU thread, and release every file descriptor opened inside the measured test.
It also proves that the watchdog can interrupt and join a guest that never emits the requested sentinel after a five-second deadline.
It does not prove OCI execution, a SOMA sandbox lifecycle, authenticated readiness, command execution, network isolation, snapshot restore, production cleanup, x86_64 support, or any latency objective.

## Identities

- SOMA Git revision: `ca08d420b01e0840458220579de75682ea30f120`.
- Outer host: Apple M3 Ultra, macOS 26.5 build 25F71, Darwin 25.5.0 ARM64.
- Apple Container version: `1.3.0`, release commit `d6de569`.
- Apple Container executable SHA-256: `6a89bf76b70f6006e59a446171e1f0f2da2e38435e33101742fbae52b16dacaa`.
- apple/containerization source revision: `2faaf9b4aff48a4745ef3d26c3f1450c1228fdf0`.
- Nested Linux kernel: `6.18.5-cz-2faaf9b4aff4` ARM64.
- Nested Linux kernel SHA-256: `65af5964da709073e1b9f575d51e082c9d3e89cff087965812b395bd7ce20e40`.
- Generated initramfs SHA-256: `9a0ead9e48b81491d954d6c668255d5a32b11d818f2a3bdb62aec96cf8e99f6e`.
- OCI image index: `docker.io/library/rust:1.98-bookworm@sha256:82150a52ec202c1b14d7817e14516c392bb7f5cfebd88f1ed531cb37ebd39922`.
- OCI ARM64 manifest: `sha256:56bfc6a715db852bdafd5e4bdf68ef7abceb791e77c47e5d87c7e861702a9ca6`.
- Rust toolchain: `1.98.0-aarch64-unknown-linux-gnu`.

The generated initramfs contains only the statically linked reviewed `arm64_init.S` fixture as PID1.
The fixture writes `SOMA_ARM64_OK` once and then waits forever.

## Invocation

The following is the executed command with three host-local absolute paths replaced by explicit placeholders.

```sh
"$APPLE_CONTAINER_BIN" run \
  --rm \
  --virtualization \
  --kernel "$KVM_KERNEL" \
  --mount "type=bind,source=${SOMA_SOURCE},target=/source,readonly" \
  --mount "type=bind,source=${KVM_FIXTURE_ROOT},target=/fixtures,readonly" \
  -w /source \
  rust:1.98-bookworm \
  sh -lc 'set -eu
printf "soma_revision="
git rev-parse HEAD
uname -srm
test -c /dev/kvm
sha256sum /fixtures/vmlinux-arm64
python3 crates/soma-kvm/tests/fixtures/build_initramfs.py /tmp/arm64-initramfs.cpio
sha256sum /tmp/arm64-initramfs.cpio
export SOMA_KVM_ARM64_KERNEL=/fixtures/vmlinux-arm64
export SOMA_KVM_ARM64_INITRAMFS=/tmp/arm64-initramfs.cpio
export CARGO_TARGET_DIR=/tmp/soma-target
timeout 60 /usr/local/cargo/bin/cargo test --locked -p soma-kvm --lib arm64::tests::boots_linux_arm64_pid1_and_releases_descriptors -- --ignored --exact --nocapture
timeout 30 /usr/local/cargo/bin/cargo test --locked -p soma-kvm --lib arm64::tests::watchdog_stops_a_guest_that_never_emits_the_expected_sentinel -- --ignored --exact --nocapture'
```

Each ignored test ran as the only selected test in its own process.
The container was disposable and exited automatically after both processes completed.

## Measured boundary

The cold-boot timer starts immediately before `boot_arm64_fixture` and stops when the function returns after observing the exact sentinel and joining the vCPU thread.
It includes fixture reads, guest-memory allocation, kernel loading, device-tree construction, KVM open and validation, VM and vCPU creation, GICv3 initialization, direct Linux boot, serial emulation, sentinel observation, and vCPU join.
It excludes Apple Container startup, OCI image resolution, Cargo compilation, test-process startup, fixture path validation, and the file-descriptor counts around the function.

The timeout timer uses the same outer boundary.
Its five-second watchdog deadline starts only after the worker has installed both its ordinary pthread signal mask and KVM's temporary run mask.

## Result

The complete command exited with status 0.
The cold-boot test observed `SOMA_ARM64_OK` and reported `elapsed_ms=2401`, `fd_before=4`, and `fd_after=4`.
The forced unreachable-sentinel test reported `elapsed_ms=5024`, `fd_before=4`, and `fd_after=4`.
The generated initramfs hash matched the reviewed input used for both tests.
The kernel hash matched the retained nested-KVM fixture.

These are one-sample diagnostic results for a cold test-only tracer bullet.
They are not a public performance benchmark and must not be compared with a prepared, restored, warm-pooled, authenticated, or external create-through-first-command result.
