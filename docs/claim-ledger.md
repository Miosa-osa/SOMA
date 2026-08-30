# SOMA claim ledger

This ledger is the single place that states what SOMA can do today.
Every row uses exactly one of the five status terms defined in [the engineering standard](standards/sota-engineering-standard.md#status-vocabulary): designed, component-tested, live-proved, integrated, production-admitted.

Rules for this table:

- A live-proved row names the exact commit the run was made on and links the retained evidence artifact.
- When the code changed after a run, the row says historical, keeps the original observation, and states the status of the current bytes.
- No row may claim a higher term than its evidence supports, and no capability is integrated or production-admitted today.

Nothing in this repository is production-admitted.
No signed admission report, admission policy, or revocation state exists yet.

## Portable surface and development backends

| Capability | Status | Evidence |
|---|---|---|
| Portable facade, CLI, Rust library, and MCP server | Component-tested | Workspace tests under `./scripts/check.sh portable` |
| Durable managed lifecycle state | Component-tested | `crates/soma-local` tests |
| Docker Backend local sandbox lifecycle | Live-proved at `08bf75e` | [Docker Node 22 local run](evidence/2026-08-29-docker-node22-local.md) |
| Apple Virtualization Backend one-shot | Live-proved at `4d10493` | [Apple Node 22 one-shot](evidence/2026-08-29-apple-node22-one-shot.md) |
| Linux KVM Backend lifecycle behind the public contract | Designed | [KVM backend integration](research/kvm-backend-integration.md); `crates/soma-local/src/backend/kvm.rs` answers every lifecycle call with a typed unavailable failure |

## Build-time pipeline

| Capability | Status | Evidence |
|---|---|---|
| Bounded verified OCI layout import | Live-proved at `4d10493` | [Node 22 OCI import](evidence/2026-08-29-node22-oci-import.md) |
| Deterministic normalized logical rootfs | Live-proved at `4d10493` | [Node 22 OCI import](evidence/2026-08-29-node22-oci-import.md); cross-host reproduction of one image revision is still open |
| Pinned PVH kernel build | Live-proved at `bc61af2` | [x86_64 PVH kernel build](evidence/2026-08-29-x86_64-pvh-kernel-build.md); cross-host reproducibility untested |
| Generation compiler phases 1 through 3 and 6 | Component-tested | `crates/soma-generation` tests; phase 4 is partial and phase 5 is absent |
| Generation Candidate cold-booting on real KVM | Live-proved at `71161ea`, historical | [First sandbox command](evidence/2026-08-29-x86_64-first-sandbox-command.md); the run used initramfs layout v2, so its `GenerationId` values are no longer reproducible |
| Generation certification, signed manifest, SBOM, revocation, publication | Designed | [Generation compiler](research/generation-compiler.md); `certify_candidate` fails closed as unimplemented |
| Template document to canonical Template Lock | Component-tested | `crates/soma-template` slice 1; the registry, resolver, and filesystem oracle are test-only seams |
| A Generation built from a Template Lock | Designed | [Template implementation map](research/template-implementation-map.md), tickets T6 through T18 |

## Machine, guest, and snapshot

| Capability | Status | Evidence |
|---|---|---|
| x86_64 machine contract, PVH cold boot to `hlt` | Live-proved at `0b43bc6` | [x86_64 halt guest](evidence/2026-08-29-x86_64-kvm-halt-guest.md) |
| Pinned kernel booted through the PVH entry to a challenge-bound sentinel | Live-proved at `45d031c` | [x86_64 PVH kernel boot](evidence/2026-08-29-x86_64-pvh-kernel-boot.md) |
| Five virtio-mmio device models on the fixed bus | Component-tested; four of five live-proved at `71161ea` | [First sandbox command](evidence/2026-08-29-x86_64-first-sandbox-command.md); the network device has run only behind the link-down loopback backend |
| Guest agent repair, authenticated vsock session, readiness probe, one bounded command, cold boot | Live-proved at `71161ea`, historical | [First sandbox command](evidence/2026-08-29-x86_64-first-sandbox-command.md); the run predates launch-page schema 3 and initramfs layout v3 |
| Snapshot format v1 codec, compatibility check, and step orders | Component-tested | `crates/soma-kvm/src/snapshot` tests |
| Live capture at the pre-launch repair point and repeated restore into authenticated Instances | Live-proved at `7c1127d`, historical | [x86_64 snapshot restore](evidence/2026-08-29-x86_64-snapshot-restore.md); the captured Generation still carried a Generation-scoped responder private key in `memory.raw`, which [ADR 0024](adr/0024-per-instance-guest-responder-authority.md) removed, so this run cannot certify current bytes |
| Capture and restore on the current per-Instance authority design | Component-tested | Recapture on current code is required by finding P1.5 of [the re-audit](reviews/2026-08-29-implementation-reaudit.md) and has not been run |
| Fresh per-Instance responder authority in launch-page schema 3 | Component-tested | `crates/soma-guest` frozen vector, hostile-page, and cross-Instance tests |
| Authenticated readiness receipt gating the restored ready transition | Component-tested | `crates/soma-kvm/src/snapshot/readiness.rs` and its restore tests |

## Host-side mechanisms

| Capability | Status | Evidence |
|---|---|---|
| Privileged network broker: sterile bundles, assignment, activation, release, reconcile | Live-proved at `bceeb7b`, historical | [Linux network profile](evidence/2026-08-29-linux-network-profile-live.md); the run predates peer authorization and receipt-gated activation |
| `soma-netd` peer authorization and single-use activation capability | Component-tested | `crates/soma-netd` authorization and activation tests; a privileged live run through `scripts/netd-live-tests.sh` has not been retained |
| Real guest networking: virtio-net attach, TAP transfer, proxy, ingress | Designed | [Linux network profile v1](research/linux-network-profile-v1.md) |
| VMM jail launcher constraining the `jail-probe` stand-in | Live-proved at `bd0234e` | [VMM jail live](evidence/2026-08-29-vmm-jail-live.md); it does not wrap the real `soma-vmm` binary |
| Jail around the real `soma-vmm` process | Designed | [VMM jail profile](research/vmm-jail-profile.md) |
| XFS reflink storage profile, sterile templates, clone, lease, reconcile | Live-proved at `f91f219` | [XFS reflink profile](evidence/2026-08-29-xfs-reflink-profile.md); the launch path does not yet consume prepared heads |
| Prepared-worker allocator, ledger, reconciliation, capacity admission | Component-tested | `crates/soma-hostd` tests; the daemon starts only with the explicitly requested development launcher |

## Performance

| Capability | Status | Evidence |
|---|---|---|
| Burst harness enforcing the benchmark contract | Live-proved at `ccf7bcf` against the Docker Backend only | [Burst harness dry run](evidence/2026-08-30-burst-harness-dry-run.md); this proves the harness, not SOMA performance |
| Admitted KVM burst campaign and the 10 ms objective | Designed | [Benchmark contract](benchmark-contract.md) and [production admission evidence](research/production-admission-evidence.md); no KVM cohort, signed report, or accepted result exists |

## What no row claims

- No end-to-end Instance receipt covering allocation through proven cleanup exists, so nothing is integrated.
- The proven KVM runs were driven by test processes through crate-internal seams, not by the `soma-vmm` binary behind the public Backend.
- Every retained latency number is a debug-build single-host observation and none of them is a benchmark result.
