# ADR 0027: Validate every Generation compatibility field as hostile input

- Status: Accepted
- Date: 2026-08-30
- Extends: ADR 0019 and ADR 0026

## Context

`verify_generation` decoded a `SOMAGEN` manifest and then checked a selected list of constants.
The decoder already rejected structural violations: unknown tags, wrong descriptor roles, unsupported media types, duplicate descriptors, non-ascending overlay templates, oversized strings, and trailing bytes.
Everything the decoder admitted as structurally valid was then accepted as semantically compatible unless it appeared in that short list.

Guest memory size and alignment, the memory-slot and launch-page layout versions, the repair policy version, the Instance time to live, the overlay minimum and maximum bounds, the relation between an overlay template's capacity and its object size, the relation between the declared network policy class and its canonical digest, and the semantics of an explicit workload probe were all unchecked.
Several of those values are what the Host uses to build a machine, so an incompatible or nonsensical value would have been acted on rather than refused.
Rejections were also untyped: every incompatibility produced the same `Unsupported` classification, so no caller could tell which invariant failed.

The implementation audit of 2026-08-29 records this as Priority 0 finding P0.5.

## Decision

Every decoded manifest field is hostile until validated, and compatibility validation is one explicit pass over all sixteen groups.

Numeric relations use checked arithmetic.
The declared artifact sizes are summed with `checked_add` and a manifest whose sizes cannot be summed is rejected before any bound derived from them is used.

Versions are validated explicitly against named constants rather than implicitly through a later failure.
`MEMORY_SLOT_LAYOUT_VERSION`, `LAUNCH_PAGE_LAYOUT_VERSION`, `REPAIR_POLICY_VERSION`, `SNAPSHOT_FORMAT_VERSION`, and `SNAPSHOT_CAPTURE_POINT_VERSION` join the existing contract constants.
`LAUNCH_PAGE_LAYOUT_VERSION` restates `soma_guest::LAUNCH_PAGE_SCHEMA_VERSION` and a test binds the two, so a Generation built for a different launch-page schema is refused instead of booting into a page its guest cannot parse.

Cross-field relationships are validated rather than fields in isolation.
The root UUID must derive from the bound tree digest.
Each overlay template's object size must equal its declared capacity, and the declared minimum and maximum capacities must equal the first and last template capacities.
The writable-storage class must name both a certified profile class and a template this Generation actually built.
The network policy class and the canonical policy digest must name the same policy, including the negative case where an explicit policy must not carry the isolated or runtime-default digest.
An explicit workload probe must be a bounded, absolute, control-free byte string.

Every failure returns one typed redacted [`Incompatibility`] naming the invariant and never the value.
`CompileError::incompatibility` exposes it, so a caller can act on the exact reason while the value that violated it is never echoed back.

`verify_generation` cannot report `launchable = true`.
A ready manifest without a certified snapshot is an integrity failure under ADR 0026, and verifying a captured snapshot is phase 5 work, so the resolution fails closed.
`verify_candidate` has no launchability field at all.

## Verification

Every manifest field has a positive case, a negative case, and, where it is a range, both boundary cases.
Cross-field cases cover the root UUID derivation, overlay capacity against object size, overlay bounds against the template list, the writable class against both the profile and the built templates, and the network class against the canonical digest.

Every truncation of a canonical manifest is rejected without panicking, and leading or trailing bytes never decode.
Every single-bit mutation of a canonical manifest is exercised: none panics, every mutation that decodes re-encodes to exactly the bytes it came from, and every mutation that passes compatibility also satisfies an independently written restatement of the invariants.
Mutations that survive are confined to content-addressed digests and sizes, the free-form provenance string, and other values inside their declared ranges; each still changes the manifest identity, and the store re-verifies every object a digest names.

A correctly encoded, fully present Candidate is rejected when the exact host profile disagrees, and the rejection names `OverlayCapacity`.
No resolution reports a launchable Generation.

## Consequences

A Host that resolves a Generation now refuses every incompatible manifest with one named reason instead of acting on an unchecked field.
`CompileError` gains one accessor and the crate gains one public enum; both are additive.

This decision does not add a signature, a revocation check, or an attestation to manifest verification.
It also does not replace the opaque workload-probe byte string with the structured command the audit asks for in P1.5; it only bounds what that byte string may contain.
