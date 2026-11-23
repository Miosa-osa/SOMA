# ADR 0009: Machine shape and reproducible customization

- Status: Accepted
- Date: 2026-08-28

## Context

Sandbox callers need to choose CPU, memory, and writable storage instead of accepting one provider-defined size.
People also need a readable name for their work, while lifecycle operations require a globally unique identity that cannot be confused with mutable presentation metadata.

Workload customization creates a second concern.
Allowing callers to mutate a shared warm VM or promote an arbitrary running filesystem would capture stale identity, credentials, entropy-derived state, and unreviewed changes.
Requiring a complete cold rebuild for every small source change would instead waste the content-addressed layering and caching already provided by OCI images.

Different development and production backends cannot enforce every dimension in the same way.
The public contract therefore needs to preserve caller intent without turning an unsupported backend property into invented evidence.

## Decision

Every one-shot run and managed launch carries one immutable requested Machine shape.
The shape contains a vCPU count, memory in MiB, writable storage in MiB, and explicit capabilities.
The contract does not expose provider instance types or billing tiers.

Every receipt carries an effective shape with separate observations for CPU, memory, storage, and capabilities.
A backend records an observed value only when it can verify that value through its accepted contract.
An unsupported or unverified dimension remains explicitly unavailable.
The facade rejects an observed value that contradicts the request.

A run or launch may carry one bounded human-readable Machine name.
The name is display metadata and participates in the canonical request fingerprint.
It never replaces the globally unique Instance ID, selects a runtime object, or proves ownership.
Execute, inspect, stop, and destroy continue to address the exact Instance ID.

Machine shape is fixed for one Instance lifetime in the first stable contract.
Changing CPU, memory, storage, image, or immutable startup configuration launches a new Instance with a new operation and Instance identity.
Live resource hotplug and in-place resize remain outside the first stable release.

OCI layers are the reproducible customization mechanism.
A changed Dockerfile or build input produces a new OCI manifest digest, and the Generation pipeline certifies that exact content as a new Generation.
Layer and Generation caches may reuse unchanged content without reusing mutable Machine state.

Mutable project data is a separate storage concern from the disposable root state of a Machine.
Future workspace and persistent-volume capabilities must have explicit ownership, size, mount, durability, snapshot, and cleanup contracts.
They cannot silently change the meaning of the requested writable-storage dimension.

The Apple Container 1.3 development backend can request and observe vCPU and memory.
Its create and run commands do not expose a root writable-layer size, so that backend reports effective writable storage as unavailable unless a later adapter adds a separately specified and transactionally owned storage resource.
The production KVM backend must enforce the requested writable-disk dimension before it can report that dimension as observed.

## Alternatives considered

### Use provider size names

This option was rejected because names such as an instance type or plan couple callers to one fleet and hide the effective CPU, memory, storage, and capability values.

### Use the display name as the Machine identity

This option was rejected because names are mutable, collision-prone, and chosen for people.
Lifecycle authority and retry safety require an opaque globally unique Instance identity.

### Allow live resizing in the first release

This option was deferred because CPU, memory, block-device, snapshot, guest-agent, and billing behavior would all need an additional state transition and recovery contract.
Launching a replacement Instance is simpler to verify and keeps Generation compatibility immutable.

### Capture arbitrary running Machines as new Generations

This option was rejected for the initial pipeline because a live filesystem and memory image can contain duplicate identity, credentials, transient state, and nondeterministic changes.
OCI-derived construction provides a reviewable and content-addressed source of truth.

### Pretend every backend enforced the requested shape

This option was rejected because a request is not evidence.
Partial effective-shape observations let development adapters remain useful without overstating their isolation or resource enforcement.

## Consequences

The CLI, Rust facade, MCP tools, receipts, and future remote protocol use the same shape and naming semantics.
Adapters translate the portable shape into backend controls and return per-dimension evidence.

Operators may offer presets as a convenience, but presets expand into the exact portable shape before admission.
Presets never enter the VMM contract.

Incremental customization benefits from OCI layer caches and Generation reuse while every mutable Machine remains single-use.
Persistent workspace support requires a separate decision before implementation because it introduces another owned resource and cleanup transaction.

## Verification

Contract tests must cover minimum and maximum accepted shapes, zero and overflow rejection, partial effective observations, backend contradictions, and serialization round trips.
Name tests must cover canonical syntax, length bounds, request fingerprint participation, duplicate display names with distinct Instance IDs, and proof that runtime ownership never depends on a display name.

Backend tests must demonstrate actual CPU, memory, and storage enforcement before reporting observed values.
Lifecycle tests must prove that changing a shape or image creates a new Instance and never mutates or reuses a previous Machine lifetime.
