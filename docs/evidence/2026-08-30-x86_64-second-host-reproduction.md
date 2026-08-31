# x86_64 second-host reproduction: kernel, rootfs, and the complete live suite - 2026-08-30

## Evidence boundary

This is the first reproduction of SOMA's Linux KVM path on a host that is not one of the
project's own development machines: an AMD laptop running a non-Ubuntu distribution, with
rootless Podman answering as Docker.

It proves four things and nothing more:

- The pinned guest kernel build is reproducible across hosts: a fresh `kernel/build.sh` run on
  this machine produced `vmlinux-6.12.107-soma-v1` with SHA-256
  `f1af3a142fa39916cfac425a01b16b5f328279823533421c9eec3f192c05b746`, byte-identical to the
  digest every retained 2026-08-30 evidence document pins. The kernel README recorded
  cross-host reproducibility as untested; for this kernel revision it is now tested once.
- The deterministic normalized rootfs reproduces across hosts for one image revision: the
  `node:22` EROFS root compiled here is
  `sha256:48a6cf92bd0b4a57ee7ea87f0d3efe774ad26bd47d6db4ed6c23c83dcfe8aa48` (1,129,172,992
  bytes), byte-identical to the root the warm-path evidence pins. The OCI import evidence
  recorded cross-host reproduction of one image revision as open; for this revision it is now
  observed once.
- All fourteen live KVM tests pass on this host: `kvm_probe` 1, `x86_64_halt_guest` 2,
  `x86_64_kernel_boot` 2, `x86_64_sandbox_boot` 2, and `x86_64_snapshot_restore` 7 (capture and
  restore through `node -v`, two-instance independence, certification, the three rejection
  proofs, and the timing loop). Earlier evidence counted thirteen; the restore suite has grown
  by one test since. A cold-booted `node:22` Generation and a restored one both returned
  `v22.23.2` through the authenticated session.
- The warm restore timing on this hardware class lands where the host-class analysis predicts:
  Ready p50 20.72 ms over ten restores, between the retained Core Ultra 9 275HX result
  (about 12 ms) and the Xeon Gold 6138 result (about 29 ms).

It does not prove a burst, a jail, prepared workers, network egress, a latency objective, or
any claim at a percentile beyond the ten samples behind it. The host carried this operator's
interactive load throughout. Nothing here is a benchmark result.

## Execution environment

- SOMA Git revision: `f6566e33f09103e8617aaf6831a3638aff4698b2`, which is `f0a2a2b` plus
  documentation-only changes (two rustdoc link fixes and a `check.sh` doc gate); no code the
  measured path executes differs from `f0a2a2b`.
- Host: AMD Ryzen 5 7520U (4 cores, 8 threads, Zen 2 class, laptop), 5.6 GiB RAM,
  NVMe storage, `/dev/kvm` accessible through a login ACL.
- Host kernel: `Linux 6.17.6-lux-amd64 x86_64`, Lux 2 (Bellatrix), a non-Ubuntu distribution.
- Container runtime: rootless Podman 5.4.2 answering the `docker` CLI. This is outside the
  documented Ubuntu-plus-Docker environment and required the two accommodations recorded under
  "Environment findings" below.
- Rust toolchain `1.98.0 (88d9e12ae 2026-08-18)`, release profile, guest agent
  `x86_64-unknown-linux-musl` release, 774,320 bytes, SHA-256
  `7eedf1744ac4e5642e6bb5541d8516dd3cb3a71b37223a5b5832c3123378378e`.
- Guest kernel: `vmlinux-6.12.107-soma-v1`, SHA-256
  `f1af3a142fa39916cfac425a01b16b5f328279823533421c9eec3f192c05b746`, built on this host.
- Filesystem tools: erofs-utils 1.9.4 and e2fsprogs 1.47.0 from `scripts/build-fs-tools.sh`.
- Images: `docker.io/library/node:22` at list digest
  `sha256:8a34c4ab3ea2c5cd194f07e317b2a8f09461d3c8b05c4e34c8ccd56d56024c4d` and
  `docker.io/library/busybox:stable-musl` at list digest
  `sha256:3c6ae8008e2c2eedd141725c30b20d9c36b026eb796688f88205845ef17aa213`, exported to OCI
  layouts with `podman save --format oci-dir` and passed through `SOMA_OCI_NODE_LAYOUT` and
  `SOMA_OCI_BUSYBOX_LAYOUT`.
- Captured Candidate:
  `sha256:d98cdc35ccc39317c042d46009b7580edc18a437724fd1f1e16cf5c09350e01e`, EROFS root
  `sha256:48a6cf92bd0b4a57ee7ea87f0d3efe774ad26bd47d6db4ed6c23c83dcfe8aa48` (1,129,172,992
  bytes), overlay template
  `sha256:ecfecc597f7dfa7b98dec28adb5eeb3a15357e090cbadf62fb1c627dc41fb790` (268,435,456 bytes),
  initramfs `sha256:c23a9d177120b588c11a348e7e55f5dc5137e5190eb795e0770bb8e5ddc8ca6e`.
- Machine shape: 1 vCPU, 1 GiB RAM, 256 MiB writable class.

## Reproduction

```sh
./scripts/build-guest-agent.sh
./scripts/build-fs-tools.sh fs-tools
PODMAN_USERNS=keep-id SOMA_KERNEL_JOBS=4 kernel/build.sh
podman save --format oci-dir -o ~/soma-oci/busybox busybox:stable-musl
podman save --format oci-dir -o ~/soma-oci/node22 node:22

SOMA_OCI_BUSYBOX_LAYOUT=$HOME/soma-oci/busybox \
SOMA_OCI_NODE_LAYOUT=$HOME/soma-oci/node22 \
SOMA_EROFS_TOOLS=$PWD/fs-tools/erofs \
SOMA_E2FSPROGS=$PWD/fs-tools/e2fsprogs \
SOMA_X86_64_VMLINUX=$PWD/kernel/out/vmlinux-6.12.107-soma-v1 \
  cargo test --locked --release -p soma-kvm \
    --test kvm_probe --test x86_64_halt_guest --test x86_64_kernel_boot \
    --test x86_64_sandbox_boot --test x86_64_snapshot_restore \
    -- --ignored --test-threads=1 --nocapture
```

## Warm restore timing

Nanoseconds since the restore began reading the manifest, ten iterations, nearest-rank
percentiles over the raw samples. With ten samples the p99 is the maximum draw, not an interior
order statistic; read it as the worst of ten.

| Milestone | p50 | min | max |
| --- | ---: | ---: | ---: |
| validate manifest | 81,424 | 75,573 | 336,695 |
| create VM | 414,782 | 351,653 | 672,477 |
| map memory privately | 463,414 | 394,443 | 722,702 |
| launch page slot mapped | 632,412 | 467,191 | 779,279 |
| register memory slots | 670,373 | 490,555 | 807,783 |
| irqchip, PIT, routes | 942,937 | 670,894 | 1,893,799 |
| devices restored | 967,884 | 693,377 | 1,928,204 |
| vCPU created | 1,070,858 | 767,316 | 2,012,693 |
| vCPU state restored | 1,155,077 | 859,360 | 2,185,959 |
| eventfds and interrupt state | 1,367,037 | 1,015,654 | 2,373,383 |
| fresh launch page written | 1,592,281 | 1,231,842 | 2,583,559 |
| device thread serving | 1,677,772 | 1,290,192 | 2,655,725 |
| resume | 1,750,449 | 1,343,071 | 2,731,357 |
| launch page consumed | 5,076,277 | 4,297,820 | 5,934,846 |
| vsock connected | 7,534,390 | 6,624,686 | 10,363,542 |
| handshake done | 15,049,023 | 12,523,363 | 18,330,508 |
| repair done | 16,682,572 | 14,013,081 | 21,355,038 |
| **ready** | **20,722,095** | **17,794,658** | **26,095,142** |
| execute done | 53,543,263 | 47,141,908 | 70,794,276 |

Three observations, none of them a claim beyond this host:

- Ready p50 20.72 ms sits between the two retained hosts (12.2 ms on a Core Ultra 9 275HX,
  about 29 ms on a Xeon Gold 6138), consistent with the per-core-speed model in
  [host class and the burst result](../research/host-class-and-burst-projection.md) for a
  budget mobile Zen 2 part.
- `execute done` p50 53.5 ms is within one percent of the Core Ultra 9 measurement (53.1 ms):
  once the guest is Ready, `node -v` startup inside one vCPU dominates and the host CPU class
  matters much less than it does before Ready.
- The pre-resume prologue is 1.75 ms here against 3.01 ms on the Core Ultra 9 measurement taken
  the same day. The samples are too few to compare hosts; what both agree on is the shape -
  `KVM_CREATE_VM` plus `KVM_CREATE_VCPU` dominate the prologue, which is the prepared-worker
  case restated.

## Environment findings

Three portability findings from running the documented flow outside Ubuntu-plus-Docker, kept
here because they cost this reproduction an hour and will cost the next operator the same:

- `kernel/build.sh` runs the builder with `-u "$(id -u):$(id -g)"`. Under rootless Podman
  answering as Docker, that maps the build to a subordinate uid that cannot read the
  bind-mounted repository, and the container fails with `Permission denied` on `/work/build.sh`.
  `PODMAN_USERNS=keep-id` restores the caller's own id inside the container and the build
  proceeds; real Docker is unaffected by that variable.
- The live tests export OCI layouts through `docker save`. Podman's Docker shim does not
  produce the layout the tests expect; `podman save --format oci-dir` does, and the tests
  accept it through `SOMA_OCI_BUSYBOX_LAYOUT` and `SOMA_OCI_NODE_LAYOUT` exactly as documented.
- The kernel.org download failed once mid-transfer with an HTTP/2 `PROTOCOL_ERROR` that
  `curl --retry 3` did not retry, because the transfer had already begun. Resuming with
  `curl -C -` completed it, and `build.sh` verified the pinned digest before using it.

## What this record does not prove

- No burst, no concurrency above one, no jail, no prepared worker, no network egress, no
  certification chain beyond the suite's own gates, and no latency objective.
- Ten samples on one loaded laptop support the reduction narrative and the host-class model;
  they do not support any endpoint as a point estimate.
- The kernel and rootfs digests are each reproduced on exactly one additional host, for exactly
  the pinned input revisions; this is one observation of reproducibility, not a proof of it.
