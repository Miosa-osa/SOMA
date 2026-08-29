# ADR 0015: Evidence construction contracts

- Status: Accepted
- Date: 2026-08-28

## Context

Portable evidence types must be easy for a backend to construct without allowing fields that belong to one request to drift apart.
The first `InspectionObservation` constructor accepted operation, instance, and workload identities as separate values even though all three came from one `InspectionRequest`.
The first `NetworkCleanupEvidence` constructor accepted seven unrelated positional values, which was difficult to review and easy to reorder.
Replacing that constructor with a uniform-only value would be simpler but would violate ADR 0012 because every network resource needs an independent cleanup disposition.

## Decision

An inspection observation is constructed from the original typed `InspectionRequest` plus the backend's observed state, network evidence, backend identity, and observation time.
The request remains the single source for operation, instance, and workload identity.

Network cleanup evidence starts from an explicit uniform disposition and exposes a named builder for every independently owned resource.
Backends can therefore report lease, runtime attachment, address lease, egress, DNS, proxy, and ingress cleanup independently without a long positional constructor.

## Consequences

The pre-alpha Rust constructor surface changes before any stable release.
Call sites become self-describing and preserve the full resource-by-resource evidence required by ADR 0012.
Adding another independently owned network resource requires a field, accessor, named builder, serialization coverage, and cleanup-terminal coverage in the same module.

## Verification

Tests construct different dispositions for every network cleanup resource and assert that each value is preserved.
Backend lifecycle tests continue to verify that inspection identity is derived from the exact request and that Apple cleanup evidence remains value-equivalent.
