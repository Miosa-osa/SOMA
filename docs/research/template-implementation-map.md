# Template implementation map

This map turns [ADR 0022](../adr/0022-compose-templates-into-generation-locks.md) into implementation slices.
Each slice produces user-visible behavior or independently testable evidence.
It deliberately avoids creating one template god module or a collection of shallow pass-through crates.

## End-to-end flow

```text
AUTHORING PLANE

Template source
    |
    v
parse and schema-check
    |
    v
resolve exact module versions and OCI platform digest
    |
    v
compose files, commands, requirements, and policy ceilings
    |
    v
authorize requested capabilities
    |
    v
emit canonical Template Lock and explanation
    |
    v
build immutable filesystem and machine inputs
    |
    v
scan, attest, boot-test, capture, and certify
    |
    v
publish immutable Generation and compatibility evidence

PLACEMENT PLANE

Template revision or Generation reference
    |
    v
resolve to one ready certified Generation
    |
    v
select compatible Host and reserve complete capacity
    |
    v
send exact Generation, shape, and narrowed policy to Host

HOST LAUNCH PLANE

claim sterile prepared resources
    |
    v
create fresh Instance identity, disk, network, and authority
    |
    v
restore Generation and repair clone identity
    |
    v
authenticate soma-guest and prove readiness
    |
    v
deliver fresh environment, secret, upload, and workspace inputs
    |
    v
start or expose the selected agent command
    |
    v
serve Execute, stdio, PTY, MCP, ingress, and inspection operations
    |
    v
timeout, disconnect, stop, checkpoint, or destroy
    |
    v
revoke authority, release resources, and emit cleanup evidence

MAINTENANCE PLANE

retain referenced Template Locks and Generations
    |
    v
revoke unsafe artifacts and stop new placement
    |
    v
garbage-collect only after leases, references, and retention permit it
```

No registry lookup, module merge, package installation, scan, signature operation, or Template authorization occurs inside the Host Launch plane.
No reusable secret, network identity, Instance identity, or writable tenant state enters a Generation or snapshot.

## Deep module seams

The architecture needs a small number of deep modules with narrow interfaces.
Internal adapters remain private until at least two real implementations require a seam.

### Template compiler

Interface:

```text
compile(TemplateSource, ResolutionContext) -> TemplateLock | TemplateDiagnostics
explain(TemplateLock) -> ResolutionExplanation
```

The implementation owns schema validation, module resolution, cycle detection, composition, conflict detection, provenance, policy ceilings, canonical encoding, and deterministic identity.
Callers do not merge module fields themselves.

### Generation builder

Interface:

```text
build(TemplateLock) -> GenerationCandidate | BuildFailure
certify(GenerationCandidate, CertificationProfile) -> Generation | CertificationFailure
```

The implementation owns OCI acquisition, normalized filesystem construction, agent and tool staging, build isolation, SBOM generation, scanning, boot and readiness tests, snapshot capture, compatibility evidence, and publication material.
It extends the existing `soma-generation` responsibility rather than entering `soma-vmm`.

### Launch-input broker

Interface:

```text
materialize(DeclaredInputs, LaunchBindings, FreshInstance) -> DeliveredInputs | InputFailure
revoke(FreshInstance) -> RevocationEvidence
```

The implementation owns required environment names, sealed values, secret references, file delivery, egress-proxy credentials, redaction, rotation state, and revocation.
It receives a fresh authenticated Instance and cannot write reusable authority into a Generation.

### Policy compiler

Interface:

```text
narrow(TemplateCeiling, LaunchRequest, OrganizationPolicy) -> EffectivePolicy | PolicyFailure
```

The implementation owns permission intersection, DNS behavior, domain and CIDR rules, protocol and port rules, ingress exposure, metadata protection, and an explanation of every allowed capability.
Launch requests cannot widen the Template ceiling.

### Agent session

Interface:

```text
open(ReadyInstance, AgentCommand, Transport) -> AgentSession | SessionFailure
```

The implementation owns direct execution, stdio, PTY, MCP bridging, disconnect behavior, background processes, bounded output, and terminal results.
Agent brands remain data supplied by modules rather than branches in the VMM.

## Ticket dependency map

```text
T1 schema
 |
 +--> T2 module packages and exact resolution
 |      |
 |      +--> T3 composition and conflict engine
 |              |
 |              +--> T4 policy authorization
 |              |
 |              +--> T5 canonical Template Lock
 |                       |
 |                       +--> T6 deterministic build plan
 |                                |
 |                                +--> T7 Generation construction
 |                                         |
 |                                         +--> T8 certification and publication
 |                                                  |
 |                                                  +--> T9 registry and distribution
 |
 +--> T10 launch environment and upload bindings
 |       |
 |       +--> T11 secret delivery and revocation
 |
 +--> T12 network and ingress policy compilation
 |
 +--> T13 workspace and volume contract
 |
 +--> T14 agent command and transport contract

T5 + T9 + T10 + T11 + T12 + T13 + T14
 |
 +--> T15 placement and Backend capability negotiation
          |
          +--> T16 complete lifecycle and cleanup
                   |
                   +--> T17 conformance, security, and supply-chain suite
                            |
                            +--> T18 end-to-end performance evidence
```

## T1: Versioned Template schema and parser

Deliver one bounded parser for `soma.template/v1alpha1` with typed diagnostics, unknown-field rejection, input-size limits, path validation, and no secret literals.
Golden examples cover a static binary, Node, Python, Claude Code, Codex, OSA, Hermes, and a generic MCP server without giving any agent privileged semantics.

## T2: Module package and resolution contract

Define immutable module identity, schema version, supported platform and capability requirements, declared filesystem effects, command contributions, environment names, network requests, readiness probes, and provenance.
Resolve every transitive module to an exact digest and reject cycles, missing versions, incompatible architectures, and unpinned dependencies.

## T3: Composition and conflict engine

Compose a flat ordered module list and produce one resolved model.
Reject conflicting file ownership, users, ports, commands, sealed environment values, mount destinations, incompatible runtime requirements, and ambiguous ordering.
Return an explanation that identifies the origin of every resulting field and permission.

## T4: Policy authorization and permission ceilings

Intersect module requests, Template limits, organization policy, caller authority, and Backend capability.
Reject unauthorized capabilities before a build or Launch is admitted.
Prove that a Launch override can narrow but cannot widen the locked ceiling.

## T5: Canonical Template Lock

Encode the complete resolved result canonically and derive a content identity.
Bind exact OCI digest and platform, module digests, build inputs, policy ceiling, command, workspace, declared launch inputs, resource defaults, lifecycle defaults, builder requirements, and provenance.
The same logical inputs produce identical lock bytes regardless of map insertion order or host filesystem order.

## T6: Deterministic build plan

Translate one Template Lock into a bounded build graph without executing it.
Reuse OCI and established build ecosystems through explicit adapters while normalizing nondeterministic timestamps, ownership, ordering, metadata, and package inputs.
The plan identifies cache keys and which outputs affect Generation identity.

## T7: Generation construction

Extend the Generation pipeline to stage module content into the normalized root, construct immutable EROFS and private-overlay inputs, include the guest agent, select the certified kernel and machine profile, and prepare snapshot inputs.
Package installation and agent setup occur only in isolated build work, never during Launch.

## T8: Certification and publication

Generate an SBOM, builder provenance, vulnerability-policy result, signatures or attestations, boot evidence, readiness evidence, snapshot compatibility, and shape-family compatibility.
Publish only complete certified immutable Generations.
Interrupted or failed work cannot become ready.

## T9: Template and Generation registry

Implement immutable revisions, human aliases, build states, logs, retries, cancellation, deletion markers, revocation, tenant authorization, retention, leases, cache distribution, and garbage collection.
Aliases resolve before placement, while Hosts receive only immutable identities.

## T10: Environment and upload bindings

Validate ordinary values, required names, sealed values, command-level overrides, launch-time uploads, destination paths, ownership, modes, sizes, and redaction metadata.
Fresh bindings are delivered only after Instance identity and authenticated control exist.

## T11: Secret delivery and revocation

Support explicit environment, file, and destination-scoped egress-proxy delivery.
Secret values never appear in Template source, locks, Generations, snapshots, build logs, diagnostics, receipts, or process listings unless a declared in-guest delivery mode inherently exposes the value to that guest.
Test rotation, expiration, revocation, proxy failure, disconnect, and cleanup.

## T12: Network and ingress policy compilation

Compile deny, allowlist, or unrestricted intent into exact IPv4, IPv6, DNS, domain, CIDR, protocol, port, ingress, proxy, and cloud-metadata behavior.
Declare TCP, UDP, QUIC, raw-socket, DNS-rebinding, redirect, SNI, Host-header, and proxy-bypass semantics explicitly.
The Host must fail closed when it cannot enforce the effective policy.

## T13: Workspace and volume contract

Separate immutable build content, bounded launch uploads, ephemeral private writable state, and separately owned persistent volumes.
Define ownership, modes, quotas, mount destinations, attach and detach authority, crash recovery, snapshot inclusion, and deletion behavior.
Never mount ambient host paths implicitly.

## T14: Agent command and transport contract

Define executable, arguments, user, working directory, environment slots, readiness probe, stdio, PTY, MCP, background process, signal, timeout, output, disconnect, and terminal-result behavior.
Prove generic agent compatibility using modules for Claude Code, Codex, OSA, Hermes, and an arbitrary stdio MCP server.

## T15: Placement and Backend capability negotiation

Resolve a Template revision to a ready Generation before host selection.
Choose only a Host that can satisfy the Generation compatibility contract, requested shape, effective policy, storage, ingress, transport, secret-delivery, and lifecycle requirements.
Unsupported requirements fail closed without a weaker fallback.

## T16: Complete lifecycle and cleanup

Wire Launch, readiness, input delivery, agent session, inspection, timeout, pause or checkpoint when supported, stop, destroy, revocation, resource release, and crash reconciliation.
Cleanup is idempotent and produces evidence even after partial Launch, client disappearance, or ambiguous transport failure.

## T17: Conformance, security, and supply-chain suite

Test hostile Template input, dependency confusion, poisoned OCI content, path traversal, module conflicts, policy widening, secret leakage, network bypass, snapshot contamination, capability mismatch, registry races, revocation, garbage-collection safety, and cross-tenant isolation.
Retain exact inputs and results for every certified Backend and agent module.

## T18: End-to-end performance evidence

Measure cold build, cached build, Generation installation, prepared Launch, first authenticated command, agent startup, 100-way burst, cleanup, registry cache miss, and fleet distribution separately.
Prove that Template convenience adds no work to the measured Host Launch boundary after placement has resolved the Generation.

## Existing ticket updates

The custom VMM decision map remains authoritative for the machine implementation.
Update its responsibilities rather than creating overlapping tickets:

- #6 consumes a canonical Template Lock when constructing a Generation.
- #8 delivers fresh declared inputs and secrets only after authenticated repair.
- #10 enforces the already authorized effective network and ingress policy.
- #13 receives an exact Generation and effective launch inputs rather than resolving Templates inside the VMM.
- #15 distributes Template Locks, Generations, revocations, and compatibility evidence across a fleet.

## Completion rule

The template system is not complete when a parser accepts a file.
It is complete only when an authored Template can deterministically produce a certified Generation, launch a fresh isolated Instance with the declared policy and inputs, run the selected agent transport, clean up completely, and return retained evidence without leaking authority or moving preparation into Launch latency.
