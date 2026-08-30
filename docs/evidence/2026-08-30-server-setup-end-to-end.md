# Existing development host setup observation - 2026-08-30

## Capability status: Component-tested

This document records an operator-driven setup observation on an existing Ubuntu KVM host.
It does not prove transformation from a fresh or empty server because the host pre-state and raw setup logs were not retained.
It does not claim Live-proved setup, cleanup, jail containment, networking, certification, or production readiness.

## Host and revision

| Field | Observed value |
| --- | --- |
| Host | One bare-metal Ubuntu 24.04.4 development host, kernel 6.8.0-138 |
| CPU class | Intel Xeon Gold 6138, 80 logical CPUs, VT-x exposed |
| Memory | 156 GB |
| Storage | 1.5 TB XFS under `/srv`, with reflink enabled |
| SOMA revision | `bfe10b2c7d149167c623bde8c8196ba02ba273b1` |
| Repository transport | Git bundle transferred without placing a GitHub credential on the host |

The host had been running before the observation.
No initial package, group, service, Rust, Docker, KVM-permission, or `/srv/soma` state was retained.
The results therefore show that the documented commands reached guest execution on that host, not that every prerequisite was installed from an empty baseline.

## Operator-observed steps

| Step | Command or script | Approximate wall time | Observed result |
| --- | --- | ---: | --- |
| 1 | Repository transfer through the original bundle procedure | Not retained | The original bundle cloned an empty working tree and required correction |
| 2 | `setup-host.sh` | About 1 minute | Ten reported readiness checks and exit zero |
| 3 | `build-soma.sh` | 3 minutes 2 seconds | CLI, guest agent, and kernel built |
| 4 | `build-fs-tools.sh` | 3 minutes 50 seconds | erofs-utils 1.9.4 and e2fsprogs 1.47.0 built from pinned source |
| 5 | `prepare-generation.sh busybox:stable-musl` | 4 minutes 7 seconds | Candidate entry published |
| 5 | `prepare-generation.sh node:22` | Not retained | Candidate entry published |
| 6 | Two one-shot KVM commands | One shell sample each | Guest commands returned successfully |

The table is a narrative reconstruction.
It is not a retained raw execution transcript and cannot support benchmark or cleanup claims.

## Guest command observations

```text
soma --backend kvm run busybox:stable-musl -- /bin/busybox uname -a
Linux soma-dd67c8b36c5d 6.12.107-soma-v1 #1 SMP PREEMPT_DYNAMIC 2026-08-29T00:00:00Z x86_64 GNU/Linux
real 0m0.773s

soma --backend kvm run node:22 -- /usr/local/bin/node --version
v22.23.2
real 0m0.677s
```

These are single shell observations of the complete one-shot commands.
They are not benchmark samples and are not accepted performance evidence.
The retained text does not contain the execution receipts or post-run resource inventory required to prove cleanup.

## Kernel digest observation

The built kernel had SHA-256 `f1af3a142fa39916cfac425a01b16b5f328279823533421c9eec3f192c05b746`.
That digest equals the value recorded in [the earlier kernel build evidence](2026-08-29-x86_64-pvh-kernel-build.md) from a different machine.
This is a two-build equality observation.
It does not prove reproducibility across builder, dependency, toolchain, configuration, or base-image changes.

## Defect found in the original bundle instructions

The original runbook used `git bundle create soma.bundle origin/main`.
A bundle made from that remote-tracking ref carried objects but no local branch a clone could check out, so the clone produced an empty working tree.

The runbook now fetches the remote, fast-forwards the real local `main` branch, verifies the bundle, and clones that branch.
The corrected procedure was reproduced separately through a complete bundle, clone, and revision comparison.
It was not the procedure used at the beginning of this host observation.

## Unresolved host responsiveness observation

The host stopped answering SSH while the larger `node:22` Candidate was being prepared.
A later reading of `/proc/pressure/io` showed a large cumulative full-stall total, but that counter covered the host's entire uptime.
No before-and-after PSI samples, per-device telemetry, process I/O accounting, or timestamped system logs were retained.

The observation is consistent with I/O contention but does not prove the cause or duration.
Preparation should remain outside the request path, and future host runs must measure PSI deltas and device behavior before deciding whether preparation requires separate storage, cgroup I/O limits, or another isolation policy.

## Evidence required for a fresh-host proof

A replacement run must retain:

- A fresh Ubuntu image or an exact pre-run package, group, service, filesystem, KVM, and work-root inventory.
- Exact commands, arguments, environment identities, start times, end times, and exit results.
- Raw bounded stdout and stderr for every step.
- CLI, guest-agent, kernel, filesystem-tool, Candidate, and Generation identities.
- Negative and failure-path results for required setup checks.
- Execution receipts and a post-run inventory proving process, descriptor, memory, storage, network, and authority cleanup.
- Before-and-after PSI, device, memory, and process telemetry for large Candidate preparation.

## What this record does not prove

- It does not prove setup from a fresh or empty host.
- It does not prove the corrected bundle procedure on the observed host.
- It does not prove certified Generation admission.
- It does not prove jail containment, networking, prepared restore, concurrency, or cleanup.
- It contains no accepted latency, throughput, density, or capacity result.
