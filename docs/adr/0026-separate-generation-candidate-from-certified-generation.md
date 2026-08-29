# ADR 0026: Separate the Generation Candidate from the certified Generation

- Status: Accepted
- Date: 2026-08-30
- Extends: ADR 0009 and ADR 0019
- Relates to: `docs/research/generation-compiler.md`

## Context

The Generation compiler design requires phases in order: resolve inputs, build the immutable root and overlay templates, build the machine artifacts, boot and capture, certify, and publish the canonical manifest last.
Phases 4 and 5, boot and capture and certification, have no implementation.

`compile_generation` nevertheless published a canonical `SOMAGEN` manifest under a `GenerationId` and returned `UnimplementedPhase` values alongside it.
The object was honestly marked non-launchable through an absent snapshot binding, but it was still a Generation manifest, stored under a Generation media type, resolvable by `verify_generation`, and named by an identity type every Launch interface accepts.
Discovery, naming, and resolution therefore all admitted incomplete material as a Generation, and only a field check separated it from a certified one.

The implementation audit of 2026-08-29 records this as Priority 0 finding P0.4.

## Decision

The compiler produces a Candidate, not a Generation.

A Candidate has its own magic, `SOMACAN\0`, its own artifact role and media type, `application/vnd.soma.generation-candidate.v1`, and its own identity type, `CandidateId`.
`CandidateId` is not `GenerationId`, so no Launch, Host, or registry interface can accept one even by mistake; a compile-fail test pins that.
`decode_manifest` accepts only ready bytes and `decode_candidate` accepts only Candidate bytes, so a party that learns a Candidate digest and relabels it as a Generation identity still cannot resolve it.

`compile_generation` returns `CompiledCandidate` and publishes the Candidate manifest last, after every artifact it references is present, with the same create-exclusive, byte-re-verifying store semantics as before.
Publishing a Candidate whose snapshot binding is not absent is rejected, so a Candidate can never carry a capture it did not perform.

`certify_candidate` runs the gates a ready Generation requires.
It re-verifies the Candidate against the exact `HostProfile` first, then fails with `CompilePhase::Certify` and `CompileErrorKind::Unimplemented` because boot, capture, and certification do not exist.
It is the only producer of `Certification`, a token with no public constructor that names the exact Candidate it certified and carries the snapshot binding the ready manifest will bind.

`promote_candidate` is the only publication path for a ready `SOMAGEN` manifest.
It requires a `Certification` for these exact bytes, rejects a token issued for other bytes, and publishes manifest-last.
A failed or revoked Candidate therefore cannot be promoted without running the gates again.

`verify_generation` keeps resolving only ready Generations.
A ready manifest with an absent snapshot is now impossible by construction and is rejected as an integrity failure, and a captured snapshot has no verifier yet, so the function cannot report `launchable = true` and fails closed instead.
`verify_candidate` is the build-side resolution and has no `launchable` field at all.

## Verification

A successful compilation publishes exactly one Candidate object and no object carrying the ready magic.
A Candidate digest relabelled as a `GenerationId` is refused by `verify_generation`.
`certify_candidate` on a real Candidate fails as unimplemented and leaves no ready object.
A build that fails before publication leaves neither a Candidate nor a ready manifest.
Two concurrent identical builders converge on one identical Candidate identity and one stored object, and still publish no ready Generation.
A certification token issued for other bytes cannot promote a Candidate, and `Certification` cannot be constructed outside the gates.

## Consequences

Nothing discoverable as a ready Generation exists until boot, capture, compatibility, security, and certification pass.
The live sandbox proof consumes the Candidate explicitly through `CompiledCandidate::candidate`, which states in the type what that boot actually exercises.
The Generation store now holds two manifest namespaces; a registry implementation must expose only the ready one.

This decision does not implement boot and capture, certification, revocation lists, or a registry lifecycle.
It creates the seam those slices must satisfy.
