# ARM64 nested KVM challenge-bound command proof - 2026-08-28

## Evidence boundary

This result proves that SOMA revision `f3e3f6520a713d5c50259a3392f72292d133461f` can direct-boot the reviewed trusted ARM64 command fixture through real nested KVM, complete an exact guest Hello, send one challenge-bound request over a dedicated control UART, execute the requested absolute program without a shell, return bounded binary output and one typed terminal result, join the sole vCPU thread, and preserve host descriptor and task baselines after return.
It also proves the reviewed boundary cases for child deadlines, process-group cleanup, aggregate output limits, a legal 64 KiB response, `execve` failure, cold boot, and forced watchdog containment.
It does not prove OCI execution, authenticated guest readiness, a SOMA sandbox lifecycle, network isolation, snapshot restore, a prepared worker, production cleanup, x86_64 support, or any latency objective.

## Identities

- SOMA Git revision: `f3e3f6520a713d5c50259a3392f72292d133461f`.
- Outer host: Apple M3 Ultra, macOS 26.5 build 25F71, ARM64.
- Apple Container version: `1.3.0`, release commit `d6de569`.
- Apple Container executable SHA-256: `6a89bf76b70f6006e59a446171e1f0f2da2e38435e33101742fbae52b16dacaa`.
- apple/containerization source revision: `2faaf9b4aff48a4745ef3d26c3f1450c1228fdf0`, tag `0.43.0`.
- Linux source: `6.18.5`.
- Linux source archive SHA-256: `189d1f409cef8d0d234210e04595172df392f8cb297e14b447ed95720e2fd940`.
- Corrected ARM64 kernel configuration SHA-256: `ae9edbe8fa7900b504e5b6d71eaf46b1cb2a7dd4b47d8f1074deb693d52efc20`.
- Corrected nested Linux kernel: `6.18.5-cz-2faaf9b4aff4-soma-uart2` ARM64.
- Corrected nested Linux kernel SHA-256: `1f750d412c3632a57c8cd6abb76bda53314bff14be5bdca24ece2b649424d0a5`.
- Deterministic command initramfs SHA-256: `b22722c32ffaedc852efda66c611f6eeccad9c095057a0e49ae22742d93cd9c9`.
- Deterministic cold initramfs SHA-256: `9a0ead9e48b81491d954d6c668255d5a32b11d818f2a3bdb62aec96cf8e99f6e`.
- OCI image index: `docker.io/library/rust:1.98-bookworm@sha256:82150a52ec202c1b14d7817e14516c392bb7f5cfebd88f1ed531cb37ebd39922`.
- OCI ARM64 manifest: `sha256:56bfc6a715db852bdafd5e4bdf68ef7abceb791e77c47e5d87c7e861702a9ca6`.
- Rust toolchain: `1.98.0-aarch64-unknown-linux-gnu`.

The command initramfs contains the reviewed statically linked PID1 agent and a separate static probe executable.
Two independent command builds and two independent cold builds were compared byte-for-byte before the evidence run.
The worktree was clean before and after the matrix, and every nested process asserted the exact short revision `f3e3f65` before running its selected test.

## Kernel prerequisite and diagnosis

The first unchanged end-to-end command test failed twice because the original reviewed kernel contained `CONFIG_SERIAL_8250_NR_UARTS=1` and `CONFIG_SERIAL_8250_RUNTIME_UARTS=1`.
SOMA advertises a diagnostic 16550 UART and a separate control 16550 UART, so Linux could not register the control device as `/dev/ttyS1` with only one configured slot.
The corrected kernel changes only those two configuration values from 1 to 2 relative to the pinned input configuration.
The unchanged end-to-end test then passed, which isolated the failure to the kernel device-count prerequisite.

The kernel build is source, configuration, and artifact identified, but it is not claimed to be byte-reproducible because the reviewed upstream builder does not pin its package snapshot or Kbuild timestamp, user, and host fields.

## Invocation

The command fixture was built twice in disposable ARM64 Linux containers with this shape.

```sh
"$APPLE_CONTAINER_BIN" run --rm \
  --mount "type=bind,source=${SOMA_SOURCE},target=/source,readonly" \
  --mount "type=bind,source=${FIXTURE_ROOT},target=/fixtures" \
  -w /source \
  rust:1.98-bookworm \
  sh -lc 'set -eu
python3 crates/soma-kvm/tests/fixtures/build_command_initramfs.py /fixtures/command-a.cpio
python3 crates/soma-kvm/tests/fixtures/build_command_initramfs.py /fixtures/command-b.cpio
cmp /fixtures/command-a.cpio /fixtures/command-b.cpio
sha256sum /fixtures/command-a.cpio /fixtures/command-b.cpio'
```

Each ignored hardware test then ran as the only selected test in its own disposable nested-KVM process.
The exact process shape below was repeated with one test name at a time.

```sh
"$APPLE_CONTAINER_BIN" run --rm --virtualization \
  --kernel "$KVM_KERNEL" \
  --mount "type=bind,source=${SOMA_SOURCE},target=/source,readonly" \
  --mount "type=bind,source=${FIXTURE_ROOT},target=/fixtures,readonly" \
  --mount "type=bind,source=${KERNEL_ROOT},target=/kernel,readonly" \
  --mount "type=bind,source=${TARGET_ROOT},target=/target" \
  -w /source \
  rust:1.98-bookworm \
  sh -lc 'set -eu
test -c /dev/kvm
test "$(git rev-parse --short HEAD)" = f3e3f65
export SOMA_KVM_ARM64_KERNEL=/kernel/vmlinux-arm64
export SOMA_KVM_ARM64_COMMAND_INITRAMFS=/fixtures/command-a.cpio
export CARGO_TARGET_DIR=/target
timeout 90 /usr/local/cargo/bin/cargo test --locked -p soma-kvm --lib arm64::tests::TEST_NAME -- --ignored --exact --nocapture'
```

The cold processes used the same wrapper with `SOMA_KVM_ARM64_INITRAMFS` set to the deterministic cold fixture.

## Verified command behavior

The host validates request bounds before it opens KVM.
The guest configures its control UART before sending an exact zero-identity Hello.
Only then does the host transmit a request containing a fresh nonzero request ID and 256-bit operating-system challenge.
Every response frame must match that identity, challenge, and exact sequence.
The ordinary console has no outcome authority after Hello and stops retaining bytes at that point.

The guest calls `execve` on one absolute path with the exact argument vector and an empty environment.
It never invokes a shell and never passes the control descriptor to the workload.
One allowance bounds stdout and stderr in aggregate.
Child deadlines and output overflow kill and reap the command process group before a terminal result is emitted.
The host accepts only exited, signaled, timed out, output limited, `execve` failed, or agent failed terminal forms with exact received-byte counts.

## Results

| Exact test process | Result | Test time | Boundary proved |
| --- | --- | ---: | --- |
| Exact argv, delayed writes, and binary payload | Pass | 6.91 s | Empty and metacharacter-bearing arguments, delayed bytes, and all byte values from 0 through 255 |
| Nonzero exit and signal | Pass | 4.53 s | Exit status 7 and `SIGTERM` remain distinct |
| Guest deadline and process-group cleanup | Pass | 7.36 s | Sleeping process, closed standard streams, and descendant pipe-holder containment |
| Aggregate output boundaries | Pass | 12.92 s | Legal 64 KiB response under terminal grace, exact 1024 bytes, limit plus one byte, and combined-stream overflow |
| Typed `execve` failure | Pass | 2.24 s | `ENOENT` is `ExecFailed`, not exit status 127 |
| Repeated command cleanup | Pass | 6.80 s | Three launches each returned to the exact host descriptor and task baselines |
| Cold PID1 boot | Pass | 2.22 s | `elapsed_ms=2221`, `fd_before=4`, and `fd_after=4` |
| Forced watchdog containment | Pass | 5.06 s | `elapsed_ms=5056`, `fd_before=4`, and `fd_after=4` |

All eight exact processes exited successfully.
The kernel and both fixture hashes were unchanged before and after the evidence run.

## Measurement warning

The command test times are Cargo test-process durations that include multiple cold VMs in several cases.
The cold timer starts immediately before the internal boot call and stops only after the expected sentinel is observed and the vCPU worker is joined.
Neither boundary is the ComputeSDK create-through-first-command boundary.

These are diagnostic results for a cold test-only tracer bullet.
They must not be compared with prepared, restored, warm-pooled, authenticated, OCI-derived, or externally measured sandbox results.
