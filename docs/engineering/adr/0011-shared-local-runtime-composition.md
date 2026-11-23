# ADR 0011: Shared local runtime composition

- Status: Accepted
- Date: 2026-08-28

## Context

The `soma` facade owns portable use-case transactions and receipt construction.
The command-line client and MCP server are separate process adapters that both need the same local backend selection, durable managed-state store, target-specific translation, and failure behavior.

Putting that composition independently in each adapter would create two lifecycle implementations.
Making the MCP server execute the CLI as a subprocess would add process and encoding overhead, weaken typed error handling, and make one user interface depend on another.
Adding Apple or KVM dependencies directly to the portable facade would also make target mechanisms leak into a library that must compile for unsupported client hosts.

## Decision

The workspace includes a `soma-local` library crate with two real consumers: `soma-cli` and `soma-mcp`.
The crate owns local runtime composition rather than a new public sandbox abstraction.

`soma-local` owns:

- Explicit and automatic local backend selection.
- Target-gated adapters from `soma-macos` and `soma-kvm` into the portable `soma::Backend` evidence contract.
- The durable file implementation of the `soma::StateStore` compare-and-swap contract.
- Runtime and state-root discovery with explicit caller overrides.
- Translation of target failures into stable portable failure classes without leaking host paths or untrusted backend text.
- One shared service used by the CLI and MCP adapters for one-shot and managed lifecycle use cases.

`soma-local` does not own command-line parsing, terminal rendering, MCP schemas, provider placement, billing, OCI Generation construction, KVM device logic, or VM lifecycle policy already owned by the facade.
The CLI and MCP layers remain shallow protocol adapters over this shared service.

Target-only dependencies remain behind Cargo target conditions.
An unsupported local host fails closed and never falls back to a host process, namespace-only execution, or an unrelated container daemon.
The future authenticated remote runtime remains a separate explicit backend rather than a hidden local fallback.

The state root is an explicit configuration value at the library boundary.
User-interface adapters may choose a documented platform default, but tests and operators can supply an isolated path.
The file store follows ADR 0010 and uses bounded opaque records, exclusive cross-process locking, synchronized temporary writes, atomic replacement, and restrictive permissions where supported.

## Alternatives considered

### Duplicate orchestration in CLI and MCP

This option was rejected because fixes for cleanup, ownership, state recovery, sizing, and receipts could diverge between human and agent paths.

### Make MCP invoke the CLI executable

This option was rejected because subprocess invocation adds unnecessary latency and converts typed library results into another textual protocol boundary.

### Put local adapters in the portable facade

This option was rejected because portable clients on Windows, Intel macOS, or unsupported Linux hosts must compile without importing another host's virtualization runtime.

### Put shared runtime code in the CLI crate

This option was rejected because MCP is not a command-line rendering feature and should not depend on a binary user's grammar or output model.

## Consequences

The workspace gains one crate because the responsibility has real depth and two independent callers.
The dependency direction is `soma-cli` and `soma-mcp` to `soma-local`, then to the portable `soma` facade and target-gated backend crates.

There is one implementation of local lifecycle composition, durable state, and target evidence mapping.
The CLI and MCP may differ in schemas and rendering while returning the same underlying execution receipts and lifecycle semantics.

The custom production KVM engine remains incomplete in the current alpha.
On Linux x86_64, local runtime diagnosis may report KVM capability, but execution stays unavailable until the real adapter satisfies the facade contract.

## Verification

Contract tests must run the same local service request through both CLI and MCP adapters and compare operation IDs, Instance IDs, receipt semantics, exact output bytes, and cleanup evidence.
Restart tests must launch in one process and inspect, execute, stop, or destroy in another process through the shared file store.
Target tests must prove unsupported hosts fail closed and target-only dependencies do not break portable compilation.
Architecture checks must keep protocol parsing, local composition, facade orchestration, and target mechanisms in their assigned crates.
