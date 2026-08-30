# SOMA claim ledger

This ledger is the single place that states what SOMA can do today.
Every row uses exactly one of the five status terms defined in [the engineering standard](standards/sota-engineering-standard.md#status-vocabulary): designed, component-tested, live-proved, integrated, production-admitted.

Rules for this table:

- A live-proved row names a commit whose relevant code is identical to the one the run was made on, and links the retained evidence artifact. Where the artifact records its own run revision, that revision is what the reader should check the row against.
- When the code changed after a run, the row says historical, keeps the original observation, and states the status of the current bytes.
- No row may claim a higher term than its evidence supports, and no capability is integrated or production-admitted today.

Nothing in this repository is production-admitted.
No signed admission report, admission policy, or revocation state exists yet.

## Portable surface and development backends

| Capability | Status | Evidence |
|---|---|---|
| Portable facade, CLI, Rust library, and MCP server | Component-tested | Workspace tests under `./scripts/check.sh portable` |
| Durable managed lifecycle state | Component-tested | `crates/soma-local` tests |
| Docker Backend local sandbox lifecycle | Live-proved at `08bf75e` | [Docker Node 22 local run](evidence/2026-08-29-docker-node22-local.md); the artifact records no run revision, so the named commit is the retention point rather than a checked run identity |
| Apple Virtualization Backend one-shot | Live-proved at `4d10493` | [Apple Node 22 one-shot](evidence/2026-08-29-apple-node22-one-shot.md) |
| Linux KVM Backend, cold-boot slice behind the public contract | Live-proved at `08e4d45` | [The KVM Backend serving one sandbox through the public command line](evidence/2026-08-30-kvm-backend-cli-run.md): `soma run` resolved a prepared Candidate, cold booted a machine, ran one command that returned `v22.23.2` from inside it, and released every owned resource. One sample |
| Linux KVM Backend, the adapter ticket #13 specifies | Designed | [KVM backend integration](research/kvm-backend-integration.md). The live slice above does not restore a snapshot, admit capacity, reserve durable operation ownership, claim a prepared worker or sterile bundle, take a network lease, activate networking, reject a conflicting request fingerprint, or reconcile after restart, and it resolves a Candidate rather than a certified Generation |

## Build-time pipeline

| Capability | Status | Evidence |
|---|---|---|
| Bounded verified OCI layout import | Live-proved at `4d10493` | [Node 22 OCI import](evidence/2026-08-29-node22-oci-import.md) |
| Deterministic normalized logical rootfs | Live-proved at `4d10493` | [Node 22 OCI import](evidence/2026-08-29-node22-oci-import.md); cross-host reproduction of one image revision is still open |
| Pinned PVH kernel build | Live-proved at `c634c89` | [x86_64 PVH kernel build](evidence/2026-08-29-x86_64-pvh-kernel-build.md); cross-host reproducibility untested |
| Generation compiler phases 1 through 3 and 6 | Component-tested | `crates/soma-generation` tests; phase 4 is partial and phase 5 is absent |
| Generation Candidate cold-booting on real KVM | Live-proved at `71161ea`, historical | [First sandbox command](evidence/2026-08-29-x86_64-first-sandbox-command.md); the run used initramfs layout v2, so its `GenerationId` values are no longer reproducible |
| Generation certification, signed manifest, SBOM, revocation, publication | Designed | [Generation compiler](research/generation-compiler.md); `certify_candidate` fails closed as unimplemented |
| Template document to canonical Template Lock | Component-tested | `crates/soma-template` slice 1; the registry, resolver, and filesystem oracle are test-only seams |
| A Generation built from a Template Lock | Designed | [Template implementation map](research/template-implementation-map.md), tickets T6 through T18 |

## Machine, guest, and snapshot

| Capability | Status | Evidence |
|---|---|---|
| x86_64 machine contract, PVH cold boot to `hlt` | Live-proved at `0b43bc6`, historical | [x86_64 halt guest](evidence/2026-08-29-x86_64-kvm-halt-guest.md); the boot and restore path changed after the run, so the current bytes are component-tested |
| Pinned kernel booted through the PVH entry to a challenge-bound sentinel | Live-proved at `45d031c`, historical | [x86_64 PVH kernel boot](evidence/2026-08-29-x86_64-pvh-kernel-boot.md); the boot path changed after the run, so the current bytes are component-tested |
| Five virtio-mmio device models on the fixed bus | Component-tested; four of five live-proved at `71161ea`, historical | [First sandbox command](evidence/2026-08-29-x86_64-first-sandbox-command.md); the run predates launch-page schema 3 and initramfs layout v3, and the network device has run only behind the link-down loopback backend |
| Guest agent repair, authenticated vsock session, readiness probe, one bounded command, cold boot | Live-proved at `71161ea`, historical | [First sandbox command](evidence/2026-08-29-x86_64-first-sandbox-command.md); the run predates launch-page schema 3 and initramfs layout v3 |
| Snapshot format v1 codec, compatibility check, and step orders | Component-tested | `crates/soma-kvm/src/snapshot` tests |
| Live capture at the pre-launch repair point and repeated restore into authenticated Instances, on the current per-Instance authority design | Live-proved at `5d71524` | [Capture and restore on the per-Instance authority design](evidence/2026-08-30-x86_64-snapshot-restore-current-authority.md): six live tests, and an object scan finding no Instance responder identity and no decodable launch page in `memory.raw`, `overlay.raw`, or `state.somasnap`. Closes finding P1.5 of [the re-audit](reviews/2026-08-29-implementation-reaudit.md). [The `7c1127d` run](evidence/2026-08-29-x86_64-snapshot-restore.md) is retained as historical: it captured a Generation whose `memory.raw` still held a Generation-scoped responder private key, which [ADR 0024](adr/0024-per-instance-guest-responder-authority.md) removed |
| Fresh per-Instance responder authority in launch-page schema 3 | Component-tested | `crates/soma-guest` frozen vector, hostile-page, and cross-Instance tests |
| Authenticated readiness receipt recording the restored ready transition | Component-tested | `crates/soma-kvm/src/snapshot/readiness.rs` and its restore tests; the receipt records the transition and gates nothing yet, because no execution or network-activation seam consumes it |

## Host-side mechanisms

| Capability | Status | Evidence |
|---|---|---|
| Privileged network broker: sterile bundles, assignment, activation, release, reconcile | Live-proved at `bceeb7b`, historical | [Linux network profile](evidence/2026-08-29-linux-network-profile-live.md); the run predates peer authorization and receipt-gated activation |
| `soma-netd` peer authorization and single-use activation capability | Component-tested | `crates/soma-netd` authorization and activation tests; the receipt binds the claiming peer and is single-use, but it is keyed by the challenge the broker gives that peer, so it is not guest evidence; a privileged live run through `scripts/netd-live-tests.sh` has not been retained |
| Bounded containment of every privileged external tool | Component-tested | `crates/soma-supervise` tests: deadline, capture ceiling, process-tree termination, and the standard-input feed bound |
| Complete broker reply delivery with operation-identity replay | Component-tested | `crates/soma-netd/src/daemon/delivery` tests; the privileged live delivery proofs in `crates/soma-netd/tests/live` have not been retained |
| Real guest networking: virtio-net attach, TAP transfer, proxy, ingress | Designed | [Linux network profile v1](research/linux-network-profile-v1.md) |
| VMM jail launcher constraining the `jail-probe` stand-in | Live-proved at `bd0234e` | [VMM jail live](evidence/2026-08-29-vmm-jail-live.md); it does not wrap the real `soma-vmm` binary |
| Jail around the real `soma-vmm` process | Designed | [VMM jail profile](research/vmm-jail-profile.md) |
| XFS reflink storage profile, sterile templates, clone, lease, reconcile | Live-proved at `f91f219` | [XFS reflink profile](evidence/2026-08-29-xfs-reflink-profile.md); the launch path does not yet consume prepared heads |
| Prepared-worker allocator, ledger, reconciliation, capacity admission | Component-tested | `crates/soma-hostd` tests; the daemon starts only with the explicitly requested development launcher |

## Performance

| Capability | Status | Evidence |
|---|---|---|
| Burst harness enforcing the benchmark contract | Live-proved at `ccf7bcf` against the Docker Backend only | [Burst harness dry run](evidence/2026-08-30-burst-harness-dry-run.md); this proves the harness, not SOMA performance |
| Admitted KVM burst campaign and the 10 ms objective | Designed, and blocked | [Benchmark contract](benchmark-contract.md) and [production admission evidence](research/production-admission-evidence.md). The harness cannot measure the KVM Backend at `a4eea45`: it drives launch, exec, and destroy as three processes, and a sandbox does not outlive the command that created it. Recorded in [the blocked burst attempt](evidence/2026-08-30-burst-against-kvm-blocked.md) |

## What no row claims

- No end-to-end Instance receipt covering allocation through proven cleanup exists, so nothing is integrated.
- The proven KVM runs were driven by test processes through crate-internal seams, not by the `soma-vmm` binary behind the public Backend.
- Every retained latency number is a debug-build single-host observation and none of them is a benchmark result.
- No capability binds network activation to evidence that guest repair completed. The activation receipt proves claimant continuity and single use only, because the broker shares no secret with the guest.
