# KVM managed-lifecycle burst blocker - 2026-08-30

## Capability status: Designed

The admitted KVM burst campaign remains Designed.
This document records a failed Host observation that identified why the current managed lifecycle cannot complete across separate command-line processes.
The raw JSONL results and complete process captures were not retained, so this document is not Live-proved evidence and contains no accepted latency result.

## Observation identity

| Field | Observed value |
| --- | --- |
| Host | One bare-metal Ubuntu 24.04.4 development host |
| Kernel | Linux 6.8.0-138 |
| CPU class | Intel Xeon Gold 6138, 80 logical CPUs, VT-x exposed |
| Memory | 156 GB total, approximately 150 GB available before the attempt |
| Storage | XFS under `/srv`, with reflink enabled |
| Backend probe | `kvm-api-12-vcpu-mmap-12288`, `probe_passed` |
| SOMA revision | `a4eea45` |
| Workload | `node:22`, three attempted samples, concurrency one |
| Classification | Unsupported cold-boot managed-lifecycle probe, not a benchmark experiment class |

The store held a `node:22` Candidate prepared before the attempted lifecycle.
The release binaries were produced through the benchmark harness controlled-build step.
Because no raw run artifact was retained, these observations support diagnosis only.

## Finding one: measured KVM children lacked runtime configuration

The initial attempt returned Backend unavailable before a machine was created.
The burst harness deliberately minimizes each child environment so ambient credentials and unrelated operator state cannot enter a measurement.
The development KVM Backend requires a prepared-store locator, a writable-head locator, and an explicit opt-in when it launches an uncertified Candidate.
Those runtime settings were absent from the child environment.

The follow-up implementation forwards only those three reviewed runtime settings for KVM runs.
It does not forward Generation-build tool paths because build preparation stays outside runtime children.
It records non-secret locator fingerprints and the effective uncertified-Candidate opt-in in run metadata.
The actual Generation identity must still come from a verified Generation and its execution receipt rather than from a path fingerprint.

## Finding two: the current command-line process owns the Machine

After runtime configuration was forwarded, the operator observed this lifecycle result:

```text
launch:  succeeded
exec:    backend unavailable
destroy: no live Instance owned by this process
```

The raw result record was not retained, so the sequence above is a diagnostic observation rather than benchmark evidence.

The harness invokes Launch, Execute, and Destroy as separate `soma` processes.
Each command opens a new `LocalRuntime` and a new KVM Backend.
The current KVM Backend owns at most one live Machine in an in-process `Option<Live>`.
When the Launch process exits, its runtime and authenticated guest session are dropped, so a later Execute process cannot address that Machine.

The one-shot `soma run` command succeeds because Launch, Execute, and Cleanup occur through one runtime in one process.
That development path does not provide the persistent ownership seam required by managed sandboxes or the ComputeSDK benchmark.

## Correct architectural consequence

The blocker must not be fixed with a benchmark-only process or a shallow global map.
The persistent Host Runtime defined by [ADR 0031](../adr/0031-persistent-host-runtime-ownership.md) must own live Instances across client process lifetimes.
CLI, MCP, and future provider adapters become clients of that runtime.
The Host Runtime composes durable idempotency, capacity admission, prepared-worker ownership, one jailed `soma-vmm` process per Machine, authenticated guest control, and proven cleanup behind one small lifecycle interface.

Making the local three-command harness addressable is necessary but not sufficient for the production performance path.
The measured path also requires certified immutable Generations, prepared restore, private writable state, fresh Instance authority, authenticated Repair and Ready, bounded resource ownership, crash reconciliation, and exact receipts.

Passing the local burst harness is not the exact upstream ComputeSDK campaign.
After the persistent Host Runtime passes its local 1, 10, and 100 concurrency ladder, a provider adapter must expose the same lifecycle to the unmodified upstream ComputeSDK benchmark and retain its exact 100-way cohort.

## Two valid next measurements

1. A `cold-boot-one-shot-development` diagnostic may run concurrent `soma run` commands today.
   It can test cold-boot correctness, host pressure, identity uniqueness, command success, cleanup, and leakage under load.
   It must retain raw samples and complete cleanup results and must never be compared with restore latency or ComputeSDK Burst TTI.
2. The admitted managed-lifecycle campaign waits for the persistent Host Runtime and the production prepared-worker path.
   Its timer begins before Launch and ends only after the declared command succeeds through the authenticated guest session.
   Destroy remains outside the TTI timer but inside cohort acceptance.

## What this record does not prove

- It contains no accepted latency, throughput, density, or capacity result.
- It does not prove a complete managed lifecycle.
- It does not prove prepared restore, certification, jail containment, networking, or cleanup.
- It does not prove the runtime-setting change through a successful KVM cohort.
- It does not replace the raw evidence required for any future Host or performance claim.
