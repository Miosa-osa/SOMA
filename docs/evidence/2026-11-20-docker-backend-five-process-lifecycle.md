# One Docker backend sandbox driven by five processes - 2026-11-20

## Evidence boundary

This run proves that SOMA revision `9665736dd39e77a6805c2c1dabb46ca5c0e6c00a` can use its Docker backend under Docker Desktop on Apple Silicon macOS to run Node 22 and Ubuntu 24.04 one-shot commands with complete cleanup, and to drive one sandbox through five separate `soma` processes by its Instance identity alone.
It is the README's "macOS with Docker Desktop" row run again at a recorded revision, which [the August record](2026-08-29-docker-node22-local.md) did not keep.
It does not prove a per-sandbox hardware VM: the Docker backend is a constrained Linux container inside Docker Desktop's Linux VM.

The same session also ran one `soma-netd` test in an emulated `linux/amd64` container, the setting issue #55 names; that result is in its own section below.

## Identities

- SOMA Git revision: `9665736dd39e77a6805c2c1dabb46ca5c0e6c00a`, `r3/soma-unified`.
- Host: Apple M3 Ultra, macOS 26.6.2, ARM64.
- Docker Desktop engine `28.5.1`, `linux/arm64`.
- Rust compiler: `rustc 1.98.0 (88d9e12ae 2026-08-18)`.
- `node:22`, observed digest `sha256:e13726ca604184d7a91fcacab1fc09eefd642dc28e79ac34e85540dd93b78cfb`, `linux/arm64/v8`.
- `ubuntu:24.04`, observed digest `sha256:9c7f200c07e3d4a03e7249d2c16bf66a51120fb876aa2242cf31451412543289`, `linux/arm64/v8`.

## Invocation

Docker Desktop had been answering HTTP 500 on every route since 21:12Z, most likely since the Mac's disk filled that afternoon.
It was restarted with `docker desktop restart` at 23:13:58Z, both images were pulled by hand first, and it was stopped with `docker desktop stop` at 23:17:26Z.
Every command used a fresh state root and `--backend docker --format json`, in the same order as [the Apple backend run](2026-11-20-apple-backend-five-process-lifecycle.md): strict doctor, Node 22 three times, Ubuntu once, then launch, exec that writes `/tmp/soma-proof`, exec that reads it, inspect, and destroy, each its own process.
Each command's full JSON envelope and receipt is retained in [the raw record](raw/2026-11-20-docker-backend-five-process-lifecycle/), with `exits.txt` holding every exit status.

## Result

The strict probe passed, and every command exited 0.
All three Node 22 runs returned exactly `v22.23.5\n`, and the Ubuntu run returned the `/etc/os-release` of Ubuntu 24.04.5 LTS.
Every receipt reported backend `docker_container`, isolation `linux_container`, on-demand preparation, 1 effective vCPU and 1,024 MiB of memory, storage not verified, a detached network with egress and DNS denied, and complete cleanup.
In the five-process lifecycle, process 3 read back exactly `written-by-process-2\n`, which process 2 had written, and after process 5 destroyed the sandbox no `soma-` container was left.

| Run | Accepted to cleanup finished |
| --- | ---: |
| Node 22, first | 1,317.6 ms |
| Node 22, second | 1,055.1 ms |
| Node 22, third | 1,040.8 ms |
| Ubuntu 24.04 | 1,011.6 ms |
| Five-process launch, accepted to ready | 912.5 ms |

These are development-backend diagnostics, not benchmarks; about 650 to 700 ms of each is image resolution through the Docker CLI.

## Issue #55 in an emulated amd64 container

The test #55 reported failing under `linux/amd64` emulation on an ARM Mac passed at the same revision, which contains #155.
Only that crate's library tests were built, in the container's own target directory, from a read-only mount of the checkout.
The image is `rust:1.98.0-bookworm` for `linux/amd64`, `sha256:82150a52ec202c1b14d7817e14516c392bb7f5cfebd88f1ed531cb37ebd39922`.

```text
$ docker run --rm --platform linux/amd64 -v "$PWD:/src:ro" -w /src -e CARGO_TARGET_DIR=/tmp/target \
    rust:1.98.0-bookworm sh -c "uname -m; cargo --version; \
    cargo test -p soma-netd --locked --lib an_absent_tool_is_a_typed_failure_rather_than_a_panic"
x86_64
cargo 1.98.0 (797e8a9bc 2026-08-05)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 33.13s
     Running unittests src/lib.rs (/tmp/target/debug/deps/soma_netd-9857052a3ab89941)
test nft::tests::an_absent_tool_is_a_typed_failure_rather_than_a_panic ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 107 filtered out; finished in 0.02s
```
