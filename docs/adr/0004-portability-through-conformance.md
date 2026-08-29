# ADR 0004: Earn portability through conformance

- Status: Accepted
- Date: 2026-08-28
- Decision owners: SOMA maintainers

## Context

SOMA's long-term direction is to become a state-of-the-art hardware-isolated sandbox engine across clouds, host classes, resource shapes, and disk sizes.
That direction can be undermined by either hardcoding one provider's plans into the VMM or claiming portability before another target satisfies the same security and lifecycle contract.

The first production target remains Ubuntu 24.04 x86_64 on bare-metal KVM hosts.
Apple Silicon macOS provides a development-only VM-per-OCI adapter and does not certify the production KVM path.
ADR 0007 extends this decision with a portable Linux, macOS, and Windows client surface plus capability-gated local and remote backends.

## Decision

SOMA separates portable Machine semantics from provider placement and host integration.
The VMM interface describes validated dimensions such as `VcpuCount`, `MemoryBytes`, and `DiskBytes` rather than branded plans or a fixed catalog of tiers.
The operator remains responsible for mapping its public resource shapes onto host capacity, CPU feature classes, NUMA policy, networking, storage, quotas, billing, and admission.

Every supported target must implement the same observable contract:

- One `soma-vmm` process owns one hardware-isolated Machine.
- A Launch binds an exact certified Generation and compatibility class.
- Memory and disk mutation remain private to one Instance.
- Clone Repair completes before user work.
- Ready requires authenticated no-op Execute completion.
- Execute and Stop preserve authentication, idempotency, ownership, and cleanup rules.
- Incompatibility fails closed without a hidden cold or weaker fallback.
- Conformance and benchmark evidence identify the exact host, artifact, runtime, and preparation class.

Resource dimensions are accepted only when they satisfy checked arithmetic, architectural alignment, Generation compatibility, device limits, and operator-supplied host limits.
SOMA must not silently round a requested dimension to a different billable or security-relevant shape.
The production terminal receipt records the effective dimensions that were actually enforced.
The Phase 0 dimension constructors reject zero and otherwise preserve their exact values.
The Phase 0 Ready value reports the Generation's exact `MachineSpec` without claiming target enforcement.
Checked arithmetic, alignment, compatibility, and host-limit enforcement begin with real target adapters and conformance tests.

Provider and cloud adapters stay outside the VMM.
An adapter may prepare cgroups, namespaces, TAP devices, disk heads, artifact handles, and process credentials, but it may not redefine SOMA's Ready or isolation semantics.
No provider name, tenant plan, cloud credential, placement zone, billing field, or warm-pool policy enters the provider-neutral Machine contract.

A new architecture, operating system, hypervisor backend, filesystem, networking mode, or cloud host class is experimental until it passes the published conformance suite.
Documentation must say which targets are implemented, tested, experimental, and unsupported.
Passing on one target does not imply support for another target.

Client portability and local engine support are different claims.
Portable code may issue the same bounded operations to a remote certified engine from Linux, macOS, or Windows even when the client machine has no supported local isolation backend.
Remote transport cannot weaken Machine semantics or turn successful compilation into target certification.

## Alternatives considered

### Provider-specific VMM builds

Separate provider builds can tune aggressively for each fleet.
They are rejected as the primary architecture because security fixes, protocol behavior, receipts, and lifecycle semantics would drift across forks.
Provider-specific launch adapters and measured host policies may vary without forking the Machine contract.

### Lowest-common-denominator portability

Restricting the design to mechanisms available on every possible host makes portability easy to claim.
It is rejected because it would discard KVM, private mapping, reflink, and host hardening capabilities that are central to security and performance.
SOMA defines strict semantics and allows target adapters to satisfy them with target-specific mechanisms.

### Fixed public resource tiers

A small enum such as `small`, `medium`, and `large` is simple for one product.
It is rejected because tier meaning belongs to provider policy and changes across clouds, CPUs, and commercial plans.
Production dimension validation preserves a stable technical contract while allowing operators to offer their own catalogs.

### Claim support from successful compilation

Cross-compilation or platform-neutral unit tests are useful development gates.
They are rejected as target support evidence because they do not exercise virtualization, restore, isolation, Repair, cleanup, or performance on the target host.

## Consequences

The initial implementation can optimize deeply for Linux x86_64 KVM without putting MIOSA-specific plans into the public seam.
Future targets need real adapters, target-host tests, compatibility metadata, security review, and published conformance evidence.
Generation artifacts may differ by architecture, CPU feature class, guest kernel, device profile, and runtime compatibility even when the higher-level workload is the same.

The portable facade and command-line tool require cross-platform compilation and behavioral tests.
Host integrations remain target-gated so a Windows client does not depend on Unix sockets, a macOS client does not link KVM, and a Linux client does not require Apple's runtime.

Portable semantics do not require identical performance across targets.
Each result must publish its exact target and experimental class.
SOMA may become broadly portable only as target evidence accumulates, not by expanding a support table ahead of implementation.

## Verification

Phase 0 contract tests cover zero dimensions and exact preservation of the Generation's `MachineSpec` in Ready.
Target conformance tests must add boundary values, checked arithmetic, unsupported alignments, Generation mismatches, and host-limit enforcement when those adapters exist.
The conformance suite must run the same Launch, Execute, Stop, authentication, Repair, isolation, failure, and cleanup behaviors against each target adapter.
Target certification must occur on representative hardware rather than only through mocks, cross-compilation, or nested virtualization.
