# Persistent MCP KVM burst on the east hosts - 2026-09-01

## Capability status: Live-proved for the current warm-cache on-demand restore path

This record proves that the public persistent MCP path can launch, execute one Node 22 command, and completely destroy real hardware-isolated KVM sandboxes across all three east hosts.
It does not prove the prepared-worker class or the 10 ms objective.

## Exact subject

The source revision was `4688224bd6fe8ffa74ce8eb42320f7286c0b3d20` from a clean checkout.
The controlled release builder produced SHA-256-bound `soma` and `soma-mcp` binaries under Rust 1.98.0.
The three hosts were `miosa-host-03`, `miosa-host-04`, and `miosa-host-10`, each running Linux x86_64 with hardware KVM and XFS storage for private reflink heads.
Every sandbox requested 1 vCPU, 1,024 MiB RAM, 10,240 MiB writable storage, the `node:22` Generation, detached networking with egress and DNS denied, and `/usr/local/bin/node --version` as the first command.
Every successful command returned `v22.23.2` with exit code zero.

The experiment class was `warm-cache-restore` because the Generation, captured snapshot, XFS template, and host page cache existed before timing.
The receipts correctly reported preparation as `on_demand`, because the public request path still constructs each restored machine after the request arrives.
Calling this `prepared-worker`, `paused-pool`, or `ready-pool` would be false.

## Measurement boundary

One initialized `soma-mcp` process served each whole host cohort.
The timer started immediately before writing the correlated `soma_launch` request and stopped immediately after parsing the correlated `soma_exec` structured response.
The measurement includes JSON-RPC write, read, correlation, parsing, sandbox launch, authenticated Ready, command execution, and command response.
It excludes MCP process start and initialization, Generation and snapshot preparation, release building, host collection, result generation, and destruction.
Destruction still ran and had to prove complete for every sample before that sample was accepted.

## Results

| Cohort | Attempts | p50 | p95 | p99 | Max | Commands | Cleanup |
|---|---:|---:|---:|---:|---:|---:|---:|
| Distributed 100 across 34/33/33 | 100 | 62.27 ms | 71.80 ms | 78.80 ms | 89.81 ms | 100/100 | 100/100 |
| `miosa-host-03`, 100 concurrent | 100 | 164.69 ms | 216.38 ms | 227.53 ms | 228.32 ms | 100/100 | 100/100 |
| `miosa-host-04`, 100 concurrent | 100 | 154.77 ms | 191.44 ms | 202.01 ms | 203.88 ms | 100/100 | 100/100 |
| `miosa-host-10`, 100 concurrent | 100 | 139.77 ms | 163.10 ms | 165.59 ms | 171.32 ms | 100/100 | 100/100 |

The distributed cohort's internal receipt milestones reached `machine_launched` at p50 6.11 ms, authenticated `ready` at p50 28.69 ms, and `command_finished` at p50 14.35 ms within the separate execution receipt.
The client-observed MCP calls were p50 34.07 ms for launch and p50 25.56 ms for execution.
Those segments use different per-operation clocks and must not be added to reconstruct the end-to-end figure.

## What this proves

- A persistent MCP client removes the two command-line process starts that dominated the earlier CLI measurement.
- The same immutable Node 22 snapshot launches on all three east hosts.
- The delivered shape is a hardware virtual machine with one vCPU, 1 GiB RAM, and a private 10 GiB writable reflink head.
- A 100-sandbox fleet burst succeeds with no failed command, shape disagreement, or incomplete cleanup.
- Each individual host can admit and complete 100 concurrent requests with no failed command or cleanup.
- After the runs, every host held zero writable head files, zero SOMA machine-host processes, and zero benchmark temporary state roots.

## What this does not prove

- It does not meet or justify a 10 ms launch-through-command claim.
- It does not exercise a shared prepared-machine pool, because the public path still restores on demand.
- It does not include attached networking, ingress, egress, DNS, filesystem transfer, PTY, secrets, or a jailed VMM composition.
- It is not the upstream ComputeSDK campaign and cannot be compared directly with a provider leaderboard whose timing boundary, client location, preparation class, and implementation are different.
- It is one image, one shape, one command, one region, and one run per reported cohort.
- The six full NDJSON files remain on the named validation hosts and are identified by SHA-256 in the retained summary, but they are not copied into this repository because they total 301 KiB for the distributed cohort alone.

## Retained artifact

[`raw/2026-09-01-east-persistent-mcp-burst/summary.txt`](raw/2026-09-01-east-persistent-mcp-burst/summary.txt) records every cohort summary, the distributed milestone statistics, post-run residue checks, and the SHA-256 identity of every full host artifact.
