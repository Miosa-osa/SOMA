## Purpose

Describe the caller-visible outcome and the single problem this change solves.

## Design

Name the module interface and seam changed by this pull request.
Link the accepted ADR when the change affects a public interface, process topology, snapshot compatibility, or trust assumption.

## Validation

List exact commands and results.
Label platform-neutral macOS results separately from Ubuntu 24.04 x86_64 and real KVM results.

- [ ] `./scripts/check.sh portable` passes.
- [ ] `./scripts/check.sh linux` passes, or this change has no Linux validation environment and the gap is stated above.
- [ ] `./scripts/check.sh security` passes.
- [ ] The self-hosted KVM smoke gate passes when this change touches KVM behavior.
- [ ] A skipped or unavailable KVM test is not represented as passing KVM evidence.

## Safety and provenance

- [ ] Tests exercise caller-visible behavior and important failure paths.
- [ ] Every unsafe block has a local `SAFETY` explanation and an auditable invariant.
- [ ] Compatibility and security checks fail closed.
- [ ] No secret, tenant data, private hostname, personal path, proprietary source, or unlicensed artifact is included.
- [ ] New dependencies and reused code satisfy Apache License 2.0 distribution and NOTICE obligations.
- [ ] Documentation distinguishes measured facts, targets, implementation decisions, and hypotheses.
