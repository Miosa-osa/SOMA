# ADR 0007: Portable client with capability-gated isolation backends

- Status: Accepted
- Date: 2026-08-28
- Extends: ADR 0004

## Context

SOMA must be usable from Linux, macOS, and Windows without pretending that those operating systems expose the same virtualization, networking, storage, or process-isolation mechanisms.
The first production engine remains a custom Ubuntu 24.04 x86_64 KVM implementation.
Apple Silicon development already has a real VM-per-OCI adapter through Apple's container runtime.
Windows does not yet have a certified local SOMA engine.

A portable command-line tool alone is not enough if callers must duplicate lifecycle and backend-selection logic.
Conversely, putting every host implementation behind one lowest-common-denominator virtualization wrapper would weaken the Linux fast path and obscure which security boundary actually ran.

## Decision

SOMA separates three concerns:

1. The portable `soma` client library owns use-case orchestration, validated requests, backend selection, stable results, and execution receipts.
2. Each local backend owns one operating system's real hardware-isolation mechanism and passes the same lifecycle conformance suite.
3. A remote backend carries the same bounded operations to a certified SOMA host so a client can use SOMA from any mainstream operating system without requiring local virtualization.

The `soma` command-line tool is a thin adapter over the portable library.
It must compile for supported Linux, macOS, and Windows Rust targets without importing KVM, Virtualization.framework, Unix socket, or shell behavior into portable modules.

Backend selection is explicit and fail closed:

- `auto` may select only a locally certified backend or an explicitly configured remote endpoint.
- `local` requires a supported local hardware-isolation backend.
- `remote` requires an authenticated endpoint and never falls back to local process or container isolation.
- A named backend such as `kvm` or `macos` either satisfies its declared contract or returns a typed capability error.

No backend may silently replace hardware isolation with a host process, namespace-only container, shared Docker Desktop VM, or weaker runtime.
Every terminal result records the backend and isolation class that actually ran.

The remote protocol will preserve bounded direct execution, idempotent operations, typed faults, cleanup semantics, and receipt evidence.
Its transport, authentication, and wire encoding require a separate protocol decision before implementation.

## Support levels

SOMA documents client and engine support separately.

| Surface | Linux | macOS | Windows |
|---|---|---|---|
| Portable CLI and library target | Required | Required | Required |
| Remote SOMA execution | Planned common path | Planned common path | Planned common path |
| Local hardware-isolated engine | Ubuntu 24.04 x86_64 KVM production target | Apple Silicon development adapter | Not implemented |
| Production performance certification | Not yet earned | Not applicable to the KVM target | Not implemented |

This table states implementation scope, not current release completion.
A target becomes supported only when its build, lifecycle, isolation, cleanup, and target-specific conformance gates pass.

The first workload format is Linux OCI images.
An image must resolve to a compatible architecture and immutable digest before execution or Generation construction.
Running Windows, macOS, FreeBSD, or arbitrary bootable guest images is outside the first stable release.

## Alternatives considered

### Compile a different public API for each operating system

This option was rejected because callers and agents would need platform branches for ordinary sandbox use cases.
Target-specific behavior remains behind the portable library instead.

### Use Docker as the universal local backend

This option was rejected because Docker availability does not prove one hardware-isolated VM per sandbox.
It would also make the effective isolation boundary depend on an external desktop or daemon configuration.

### Promise local execution on every operating system immediately

This option was rejected because unsupported virtualization mechanisms cannot be made trustworthy through API design.
Remote execution supplies universal reach while local engines earn support independently.

### Put remote transport inside `soma-vmm`

This option was rejected because the per-Machine VMM must not own public networking, credentials, fleet routing, or retries.
The portable client and operator boundary own remote transport.

## Consequences

The public library remains portable while each engine can use the strongest native primitives available on its host.
The same caller code can choose local development, certified local production, or remote execution without changing use-case semantics.

SOMA must maintain a compile matrix and a behavioral conformance matrix.
Cross-compilation proves only source portability.
Target support still requires real target-host execution.

An unsupported local operating system receives a precise capability fault and a documented remote option.
It never receives an unannounced weaker sandbox.

## Verification

Continuous integration must compile and test portable crates on Linux, macOS, and Windows.
Target-only dependencies and imports must be gated at the dependency and module boundaries.
CLI parsing, JSON output, exit codes, redaction, and unsupported-backend errors must use the same golden tests on every supported client target.

Local backend certification must run on representative hardware.
Remote conformance must prove bounded messages, authentication, idempotent replay, disconnect recovery, output limits, timeout enforcement, receipt preservation, and cleanup after ambiguous client outcomes.
