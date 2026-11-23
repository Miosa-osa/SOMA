# ADR 0032: Snapshot state binds the Candidate and every restore artifact

- Status: Accepted
- Date: 2026-08-30
- Extends: ADR 0002, ADR 0026, and ADR 0030

## Context

Generation construction publishes a Candidate before it boots and captures the prepared machine.
The ready `GenerationId` is derived only after the captured artifact descriptors are inserted into the final `SOMAGEN` manifest.

Snapshot schema 1 labeled its embedded pre-capture identity as `GenerationId`.
That identity could only be the Candidate identity because the ready Generation did not exist yet.
Requiring it to equal the final `GenerationId` would create a circular hash dependency because the final identity covers the state artifact that would contain that same final identity.

The ready Generation binding also named `memory.raw` and `state.somasnap` but omitted `overlay.raw`.
Restore requires all three artifacts, so the omitted writable overlay could not be authenticated through the Generation identity.

## Decision

Snapshot schema 2 embeds the exact `CandidateId` whose immutable machine artifacts were booted and captured.
Certification must reject a snapshot whose embedded Candidate identity differs from the Candidate being promoted.
The final `GenerationId` is then derived from the ready `SOMAGEN` manifest after the snapshot descriptors are present.

The captured snapshot binding contains three typed, content-addressed descriptors in this order:

1. `memory.raw` as `MemorySnapshot`.
2. `overlay.raw` as `OverlaySnapshot`.
3. `state.somasnap` as `StateManifest`.

The Generation manifest schema is version 2 because adding the overlay descriptor changes canonical bytes.
Snapshot schema 1 and Generation manifest schema 1 fail closed rather than being interpreted with the new semantics.

## Identity flow

```text
immutable build inputs
        |
        v
SOMACAN v2 bytes -> CandidateId
        |
        | boot and quiesce this exact Candidate
        v
SOMASNP v2 state binds CandidateId
        |
        | certify memory + overlay + state
        v
SOMAGEN v2 binds all three descriptors
        |
        v
GenerationId = sha256(exact SOMAGEN v2 bytes)
```

## Consequences

Certification has a non-circular identity to compare before promotion.
Every byte required by restore is covered by the final Generation identity.
Existing pre-alpha snapshot and Generation artifacts must be rebuilt because their schemas are intentionally rejected.
Restore receipts may report both the source Candidate identity and the final Generation identity supplied by the admitted Generation record, but they must not relabel one as the other.

## Verification gates

- Snapshot decoding rejects schema 1 and a zero Candidate identity.
- Every encoded snapshot round trip preserves the exact Candidate identity.
- Generation decoding rejects schema 1.
- The captured Generation binding requires nonzero memory, overlay, and state descriptors with their exact roles.
- Certification rejects any mismatch between the Candidate and the snapshot header.
- Restore admission independently verifies the three descriptor sizes and digests before the Generation is installed.
