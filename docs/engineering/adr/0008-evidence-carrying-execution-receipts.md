# ADR 0008: Evidence-carrying execution receipts

- Status: Accepted
- Date: 2026-08-28

## Context

Sandbox APIs usually return an identifier, output, and status while leaving the caller to trust undocumented backend behavior.
That is insufficient for AI-agent execution, evaluations, CI, incident review, and performance comparison.
Those use cases need to know which immutable workload ran, which isolation boundary enforced it, which resources were effective, when readiness occurred, what command outcome was observed, and whether cleanup completed.

SOMA already models ordered milestones and cleanup evidence.
The complete product needs one stable outcome that carries those facts across local and remote backends without exposing secrets or host internals.

## Decision

Every SOMA use case produces a versioned execution receipt.
The receipt is a domain result, not a log line and not a provider-specific response envelope.

The first encoded receipt includes:

- Receipt schema version and SOMA build identity.
- Operation, Instance, and Generation identities.
- Resolved OCI manifest digest and platform when an OCI image initiated the Generation.
- Backend identity and effective isolation class.
- Effective vCPU, memory, storage, and declared capability values.
- A canonical request fingerprint that omits raw secrets and does not expose command content by default.
- Ordered lifecycle milestones with monotonic elapsed times relative to request acceptance.
- Preparation class such as on-demand, prepared worker, paused lease, or already-Ready lease.
- Terminal command status and bounded stdout and stderr metadata.
- Cleanup state for every SOMA-owned resource class.
- Measurement-boundary metadata needed to distinguish server create, first command, and external end-to-end timing.

The receipt must identify facts the backend actually enforced.
It must never infer KVM, snapshot restoration, private copy-on-write storage, or cleanup from a requested backend name.

A basic receipt is structured evidence from the executing SOMA instance.
It is not described as cryptographic attestation.
A later verifiable profile may canonicalize and sign the receipt with a host identity and may attach hardware attestation evidence.
Unsigned, signed, and hardware-attested receipts must remain distinct evidence classes.

Raw environment values, credentials, registry tokens, guest secrets, host paths, kernel command lines, and unsanitized errors are prohibited.
Callers may opt into bounded output retention under the existing execution limits.
The canonical request fingerprint lets a caller correlate an operation without revealing its complete command to every receipt consumer.

Cleanup evidence is explicit per owned resource class and distinguishes complete, incomplete, not owned, and unsupported verification.
A signed cleanup statement remains an assertion by the executing host unless an independent verifier confirms the underlying resources.

## Use-case value

- Agent runs can prove which image digest and isolation class produced an artifact.
- Evaluations can reject samples that used a different preparation class or missed cleanup.
- CI can retain a machine-readable provenance record beside build output.
- Operators can resolve ambiguous retries by operation identity and receipt fingerprint.
- Benchmark reports can be regenerated from raw milestone data without mixing incompatible timing boundaries.
- Security review can distinguish a real target backend from a mock, cross-build, or development adapter.

## Alternatives considered

### Return only provider identifiers

This option was rejected because provider identifiers do not establish workload identity, isolation semantics, lifecycle ordering, or cleanup.

### Depend on logs as evidence

This option was rejected because logs are unstable, high-volume, backend-specific, and likely to contain sensitive data.
A receipt is bounded, versioned, and intentionally safe to retain.

### Sign every receipt in the first alpha

This option was deferred because signing without a defined trust root, rotation policy, canonical encoding, and verifier would create a misleading security claim.
The schema preserves room for a later verifiable profile.

### Put receipt construction in every backend

This option was rejected because schema rules, redaction, measurement classes, and compatibility would drift.
Backends provide typed observations while one portable receipt module validates and encodes them.

## Consequences

The portable library owns receipt assembly and schema compatibility.
Backends must expose enough typed evidence to populate the receipt but must not control its public shape.

The receipt becomes part of SOMA's stable compatibility surface at `1.0.0`.
Schema evolution therefore requires fixtures, round-trip tests, unknown-field policy, redaction tests, and explicit version review.

Receipt production adds bounded work to the launch path.
The performance budget reserves a publication stage, and encoding must avoid unbounded allocation or synchronous external storage.

## Verification

Tests must prove deterministic canonical fingerprints, stable schema fixtures, secret redaction, bounded output metadata, monotonic milestone ordering, backend truthfulness, preparation-class separation, idempotent replay, and complete cleanup classification.
Golden fixtures must cover Linux KVM, macOS development, remote execution, unsupported backends, failed launch, failed command, timeout, output limit, forced stop, and incomplete cleanup.
Performance tests must measure receipt assembly and encoding inside the declared publication budget.
