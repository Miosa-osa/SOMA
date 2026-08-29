# SOMA overall engineering assessment

- Date: 2026-08-29
- Repository revision at assessment start: `3fa1cb4`
- Status: Strong pre-alpha foundation with critical integration work remaining

This assessment applies the [SOMA state-of-the-art engineering standard](../standards/sota-engineering-standard.md) to the current repository.
It is a dated engineering judgment rather than a permanent claim.

## Summary

SOMA is a legitimate custom sandbox implementation effort rather than a wrapper presented as a VMM.
The repository contains serious work across KVM boot, OCI ingestion, filesystem construction, guest protocols, snapshot formats, networking, storage, jailing, Template locking, lifecycle contracts, and evidence collection.

The architecture is stronger than the integration state.
The project currently resembles a set of unusually well-developed machine and platform modules converging on one sandbox, not yet a fully certified production sandbox.

## Current qualitative assessment

| Dimension | Assessment | Reason |
|---|---|---|
| Architectural direction | Strong | One VM, one owner, immutable Generation, fresh Instance, explicit lifecycle |
| Modularity | Strong | Focused modules and shallow composition roots are generally preserved |
| Technical ambition | Exceptional | Custom VMM, guest, Generation, network, storage, jail, and Template work |
| OCI and filesystem safety | Strong | Extensive hostile archive, path, link, whiteout, metadata, and bound handling |
| KVM machine floor | Strong | Real x86_64 PVH kernel boot with retained evidence |
| Guest protocol | Strong design | Authentication, replay resistance, ownership, accounting, and poisoning are explicit |
| Guest-agent integration | Partial | A first command has run, but several authority and resource guarantees remain under review |
| Generation pipeline | Partial | Deterministic artifacts exist, while certification and publication semantics need closure |
| Networking | Promising | Host mechanisms and live evidence exist, while full VMM attachment and policy hardening remain |
| Storage | Promising | Reflink evidence supports prepared heads, while complete lifecycle integration remains |
| VMM jail | Promising | Strong mechanism, still requiring complete sandbox-path and independent review evidence |
| Template system | Early but rigorous | Canonical lock work exists, while later registry, build, policy, and agent slices remain |
| Security completeness | Insufficient for production | Audit blockers remain around authority, bounds, validation, and exact platform contracts |
| End-to-end lifecycle | Partial | First real command is important, but restore, devices, isolation, and cleanup gates remain |
| Performance leadership | Unproven | No authoritative prepared 100-way Launch and first-command result exists yet |
| Production readiness | Pre-alpha | Repository correctly states that untrusted production use is unsupported |

## Strongest work

### Architecture

The project consistently distinguishes Template, Template Lock, Generation, Snapshot, Machine, Instance, Backend, Launch, Execute, Stop, Destroy, and Receipt.
This vocabulary supports ownership and prevents one word such as sandbox from hiding every layer.

### Modularity

The code generally follows deep module principles.
Low-level mechanisms are separated without exposing every internal seam publicly.
The module map and source-size rules discourage god files.

### Evidence discipline

Most evidence documents state what a result does not prove.
That habit must remain mandatory because low-level milestones are easy to misrepresent as complete sandbox readiness.

### Real implementation progress

The repository has moved beyond diagrams into real KVM execution, a real guest kernel, deterministic filesystem work, a PID 1 agent, host networking, XFS reflink experiments, jail construction, and Template Lock encoding.
That breadth is valuable when each seam is closed before production admission.

## Primary weakness

The project advances several foundational tracks concurrently before every lower security and integration seam is closed.
This produces impressive breadth but increases the risk that later modules encode assumptions that an earlier audit invalidates.

The correction is not to reduce ambition.
The correction is to enforce dependency-ordered closure:

```text
authority
-> resource bounds
-> publication state
-> compatibility validation
-> real device and guest integration
-> complete lifecycle
-> isolation and recovery
-> prepared fast path
-> performance proof
-> fleet scale
```

## Immediate standard

The current implementation audit in [2026-08-29-implementation-audit.md](2026-08-29-implementation-audit.md) defines the correction gates for the reviewed commit range.
Newer commits must receive their own fixed-range review rather than being assumed correct or incorrect from the earlier audit.

No subsystem should be called complete merely because its internal tests pass.
Completion requires the applicable admission rows from the SOTA standard, including the real production seam, failure behavior, cleanup, isolation, and evidence.

## What success looks like

SOMA becomes state of the art when it can demonstrate all of the following together:

- Arbitrary supported OCI workloads become deterministic certified Generations.
- One Generation produces many fresh Instances without shared mutable identity or authority.
- A jailed custom VMM restores one Instance with private memory, disk, network, and authenticated control.
- Hostile workloads cannot escape, exhaust unbounded Host or guest resources, bypass policy, or survive cleanup.
- Every failure returns typed evidence and leaves ownership reconcilable.
- Templates and agents remain modular authoring layers rather than VMM complexity.
- The prepared path reaches the declared p50 and p99 targets without hiding work or reusing tenant state.
- A 100-way `node:22` burst completes every first command and cleanup with retained evidence.
- Additional Hosts and clouds earn support through the same conformance standard.
- Operators and agents use a small stable interface without understanding KVM internals.

## Recommendation

Continue the current architectural direction.
Preserve the agent's strong modularity, test discipline, and evidence boundaries.
Temporarily prioritize closure over breadth whenever a security, ownership, compatibility, or cleanup seam remains open.

The correct instruction to every implementation agent is:

> Build the smallest complete vertical slice that crosses the real production seam, close every lower invariant it depends on, retain the evidence, and only then widen the system.
