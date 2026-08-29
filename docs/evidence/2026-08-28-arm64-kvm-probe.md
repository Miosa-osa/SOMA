# ARM64 nested KVM development probe - 2026-08-28

## Evidence boundary

This result proves that the public `soma-kvm::probe()` interface can use a real nested ARM64 `/dev/kvm`, verify its capability contract, create an empty VM, and release the descriptors it opened.
It does not prove guest boot, command execution, sandbox isolation, snapshot restore, or launch latency.

## Identities

- SOMA Cargo source SHA-256: `e7761f0cfd6fe477568930b512e4b27a24974b59370cd40538891e527de923ca`.
- SOMA Git revision: unavailable because the new repository did not yet have an initial commit.
- Apple Container version: `1.3.0`.
- Apple Container executable SHA-256: `6a89bf76b70f6006e59a446171e1f0f2da2e38435e33101742fbae52b16dacaa`.
- apple/containerization source revision: `2faaf9b4aff48a4745ef3d26c3f1450c1228fdf0`.
- Inner Linux kernel: `6.18.5` ARM64 with `CONFIG_VIRTUALIZATION=y` and `CONFIG_KVM=y`.
- Inner Linux kernel SHA-256: `65af5964da709073e1b9f575d51e082c9d3e89cff087965812b395bd7ce20e40`.
- OCI image index: `docker.io/library/rust:1.98-bookworm@sha256:82150a52ec202c1b14d7817e14516c392bb7f5cfebd88f1ed531cb37ebd39922`.
- OCI ARM64 manifest: `sha256:56bfc6a715db852bdafd5e4bdf68ef7abceb791e77c47e5d87c7e861702a9ca6`.
- Guest architecture: `aarch64`.

The Cargo source digest uses `benchmarks.local_alpha.provenance.source_fingerprint` over the locked Rust build inputs and every file under `crates/`.

## Invocation

The following is the exact command with three host-local paths replaced by explicit placeholders.

```sh
"$APPLE_CONTAINER_BIN" run \
  --name soma-kvm-arm64-review-20260828 \
  --rm \
  --virtualization \
  --kernel "$KVM_KERNEL" \
  --cpus 4 \
  --memory 8G \
  --mount type=bind,source="$SOMA_SOURCE",target=/source,readonly \
  rust:1.98-bookworm \
  bash -c 'set -euo pipefail; cd /source; printf "guest_arch="; uname -m; test -c /dev/kvm; export CARGO_TARGET_DIR=/tmp/soma-target; /usr/local/cargo/bin/cargo clippy --locked -p soma-kvm --all-targets -- -D warnings; /usr/local/cargo/bin/cargo test --locked -p soma-kvm --all-targets; /usr/local/cargo/bin/cargo test --locked -p soma-kvm --test kvm_probe -- --ignored --exact opens_dev_kvm_and_reports_required_capabilities --nocapture'
```

## Result

The command exited with status 0.
Strict Clippy completed without warnings.
The normal ARM64 suite passed two unit tests and one public target test.
The explicitly selected live test passed one test in 0.02 seconds.
The live test compared `/proc/self/fd` counts before and after the public probe and found no leaked descriptor.
The disposable proof guest was removed.
Only Apple Container's pre-existing reusable BuildKit guest remained after cleanup.

The 0.02-second test duration is test-harness time for capability verification and empty-VM creation.
It is not a SOMA sandbox startup measurement.
