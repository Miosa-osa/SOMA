# ADR 0022: Compose Templates into immutable Generation locks

- Status: Accepted
- Date: 2026-08-29
- Extends: ADR 0009, ADR 0018, and ADR 0019

## Context

Users need a simple way to prepare sandboxes for Claude Code, Codex, OSA, Hermes, MCP servers, language runtimes, and arbitrary workloads.
They also need configurable resources, commands, workspaces, environment values, secret delivery, network permissions, and lifecycle behavior.

Putting those concerns directly in the VMM or resolving them during Launch would enlarge the trusted core and make millisecond Launch depend on registries, package managers, mutable tags, and policy composition.
A single oversized template object would also couple unrelated concerns and make reuse or validation difficult.

## Decision

A Template is a user-authored preparation recipe.
It composes one base workload with an ordered flat list of focused modules plus explicit command, workspace, environment, secret, network, lifecycle, and resource contracts.
Version 1 does not support nested template inheritance.

Resolution validates the complete composition and produces a canonical Template Lock.
The lock contains exact OCI digests, module identities, platforms, builder inputs, policy inputs, and every content-affecting value.
Offline construction and certification turn that lock into an immutable Generation.

Launch consumes an exact certified Generation and fresh Instance inputs.
Launch does not resolve a Template, pull an image, install an agent, invoke a package manager, or merge modules.

Modules may be reused across Templates.
They do not share mutable Instance state.
Conflicting exclusive fields, filesystem ownership, commands, ports, or sealed environment values fail resolution with module-specific evidence.

Template network policy is a maximum permission envelope.
Launch may narrow it but may not widen it without a separate authorization decision.
Agent and MCP modules default to denied ingress and denied egress.

Templates and locks contain secret references rather than secret values.
Delivery may use a fresh guest environment value, a fresh guest file, or host-side destination-scoped egress injection.
Reusable secrets never enter a Generation or snapshot.

Agent modules use one common module contract.
No agent brand receives privileged VMM behavior.
Multiple agents may coexist when their resolved requirements do not conflict, but the Template declares one default command.

## Verification

The first implementation slice must parse one versioned Template, reject unknown or conflicting fields, resolve every mutable OCI reference to an exact platform digest, and emit a deterministic canonical Template Lock.
Repeated resolution of the same inputs must produce identical lock bytes and identity.

Hostile tests must cover cyclic module references, unpinned transitive input, conflicting file ownership, conflicting commands, secret literals, policy widening, invalid resource values, absent executables, and unsupported lifecycle actions.

Generation construction must bind the Template Lock identity into certification evidence.
Launch tests must prove that no Template resolution or build operation occurs on the critical path.

## Consequences

Users gain small reusable building blocks without turning templates into a second orchestration platform.
The VMM and KVM crates remain unaware of agent brands, Dockerfile syntax, registries, package managers, and organization policy.
The same module can support local development and remote production when both Backends certify the required capabilities.

Template resolution and lock construction belong in the preparation plane beside `soma-generation`.
The complete product design is documented in [SOMA template system](../architecture/template-system.md).

This decision does not yet implement parsing, resolution, building, publication, secret storage, egress injection, or a ready Generation.
