# Apple Node 22 one-shot sandbox validation - 2026-08-29

## Evidence boundary

This result proves that SOMA revision `f9a7e1be615d4681c72136912fb7daf117e24d8f` can use its Apple development backend to resolve `node:22`, launch an on-demand hardware virtual machine, execute Node without a shell, return bounded output, and complete owned cleanup.
It does not prove the custom SOMA VMM, an OCI-derived SOMA Generation, cryptographic guest identity, production isolation, snapshot restore, a prepared worker, or any 10 millisecond target.

## Identities

- SOMA Git revision: `f9a7e1be615d4681c72136912fb7daf117e24d8f`.
- Host: Apple M3 Ultra, macOS 26.5 build 25F71, Darwin 25.5.0, ARM64.
- Apple container CLI: `1.3.0`, release build, commit `d6de569`.
- Rust compiler: `rustc 1.98.0 (88d9e12ae 2026-08-18)`.
- Cargo: `cargo 1.98.0 (797e8a9bc 2026-08-05)`.
- Requested image: `node:22`.
- Resolved OCI index: `sha256:8a34c4ab3ea2c5cd194f07e317b2a8f09461d3c8b05c4e34c8ccd56d56024c4d`.
- Resolved OCI manifest: `sha256:2f22d3b5ec6552b890773a152030b1360d35da0c4369799319523ccdb2d78e0e`.
- Resolved OCI platform: `linux/arm64/v8`.

## Invocation

The strict backend probe and one-shot command ran from a clean worktree at the recorded revision.

```sh
cargo run --locked -q -p soma-cli -- doctor --strict
cargo run --locked -q -p soma-cli -- \
  --format json run node:22 -- /usr/local/bin/node --version
```

## Result

The strict probe passed and reported the runtime ready but not production ready.
The command exited with status 0 and returned exactly `v22.23.2\n` on stdout with no stderr.
The requested machine shape was 1 vCPU, 1,024 MiB of memory, and 10,240 MiB of storage.
The backend observed one effective vCPU and 1,024 MiB of memory, but it did not verify effective storage size.
The backend reported hardware-VM isolation and on-demand preparation as basic backend observations.
The requested and observed network state was detached, with egress denied, DNS denied, no guest addresses, and no published ports.
The receipt recorded complete machine, memory, storage, runtime-attachment, address-lease, egress-policy, DNS-policy, ingress-binding, and guest-authority cleanup.

| Milestone | Elapsed from accepted request |
| --- | ---: |
| Request accepted | 0 ns |
| Workload resolved | 691,659,917 ns |
| Request admitted | 691,715,542 ns |
| Machine launched | 1,692,423,459 ns |
| Command-ready | 1,709,651,917 ns |
| Command started | 1,709,716,667 ns |
| Command finished | 1,803,216,417 ns |
| Cleanup started | 1,803,235,834 ns |
| Cleanup finished | 1,995,378,542 ns |

The complete accepted-request through cleanup boundary was 1.995378542 seconds.
The machine-launched through command-ready interval was 17.228458 milliseconds.

## Measurement warning

This is a real hardware-VM development-backend result, but it is not a custom SOMA VMM result.
The complete boundary includes cached image resolution, on-demand machine launch, command execution, and cleanup.
The backend classified the evidence as `basic_backend_reported`, and image digest binding remained observed-only because Apple container 1.3 cannot launch the local image by an immutable digest reference.
The result is diagnostic evidence and must not be compared with a prepared, restored, warm-pooled, externally measured, or ComputeSDK benchmark result.
