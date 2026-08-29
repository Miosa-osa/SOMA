# Contributing to SOMA

All implementation work is reviewed against the [SOMA state-of-the-art engineering standard](docs/standards/sota-engineering-standard.md).
Passing unit tests alone does not admit a capability when its real production seam, failure behavior, isolation, cleanup, compatibility, or evidence gates remain open.

SOMA by MIOSA welcomes security research, design review, documentation, tests, performance work, and implementation contributions.
The project is pre-alpha and is not safe for untrusted production workloads.

## Project scope

SOMA is a provider-neutral VMM and launch runtime for fast hardware-isolated Linux execution.
The initial production target is Ubuntu 24.04 x86_64 with KVM.
Apple Silicon macOS is a supported development environment for platform-neutral checks and the development-only VM-per-OCI adapter, but it cannot certify the Linux KVM execution path.
The portable library and command-line tool target Linux, macOS, and Windows, while local engines require separate target evidence.

SOMA does not own provider placement, tenant policy, billing, public sandbox semantics, OCI registry credentials, or fleet warm-pool policy.
Changes that introduce those concerns into the public interface should be proposed in an architectural decision record before implementation.

## Before contributing

Read [MISSION.md](MISSION.md), [NOTES.md](NOTES.md), [the glossary](GLOSSARY.md), [the architecture diagrams](docs/architecture/diagrams.md), [the threat model](docs/threat-model.md), and the accepted decisions in [docs/adr](docs/adr).
Search existing issues and pull requests before opening duplicate work.
For a significant interface, snapshot-format, process-topology, device-surface, or trust-model change, open a design issue before writing the implementation.

Security vulnerabilities must follow [SECURITY.md](SECURITY.md) rather than a public issue.
Support questions follow [SUPPORT.md](SUPPORT.md).

## Design rules

- Preserve the provider-neutral `Launch`, `Execute`, and `Stop` interface.
- Keep one `soma-vmm` process per Machine and preserve the constrained node-local allocator boundary accepted in ADR 0006.
- Keep complex ordering, rollback, compatibility, and Repair rules behind a deep module.
- Put seams where behavior actually varies and keep adapters smaller than the behavior they protect.
- Exercise a module through the same interface used by its callers.
- Do not expose host paths, TAP names, device names, arbitrary descriptors, or provider credentials as casual public strings.
- Do not add a cold-boot or compatibility downgrade behind a warm-restore request.
- Do not report Ready before authenticated Repair and a successful authenticated no-op Execute probe.
- Do not create generic `utils`, `helpers`, `common`, `manager`, or `core` dumping grounds.
- Keep authored source files below 500 lines and split by cohesive responsibility before reaching that limit.
- Explain every unsafe Rust block with a `SAFETY` comment that states the complete invariant.
- Treat guest input, snapshot bytes, Generation manifests, queue descriptors, lengths, offsets, and protocol frames as hostile.

## Development workflow

Create a focused branch from the current default branch.
Make the smallest coherent change that crosses the real public seam and proves one behavior end to end.
Add a failing test before implementation when changing behavior.
Keep commits reviewable and do not mix broad formatting changes with functional work.
Do not manually edit generated files or changelogs marked as generated.

Run the repository checks documented by the current workspace before requesting review.
At minimum, a Rust change is expected to pass the equivalent of:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Run platform-neutral checks on macOS when useful, but label them accurately.
A change that touches KVM, Linux memory mapping, seccomp, namespaces, cgroups, TAP networking, reflinks, or host cleanup also requires Ubuntu 24.04 x86_64 validation.
Do not replace a missing Linux test with a macOS result.
Portable client changes also require Linux, macOS, and Windows compile and behavior checks once those jobs are available.

## Testing expectations

Contract tests must use the semantic public `Launch`, `Execute`, and `Stop` interface rather than private implementation calls.
Encoded-transport conformance becomes a separate required gate only after a protocol codec exists.
Behavioral changes should include failure tests, rollback tests, idempotency tests, and typed fault assertions where applicable.
Snapshot changes require corruption, incompatibility, identity-repair, and cross-Instance isolation coverage.
Guest-facing parsers and device paths require fuzz targets or a documented reason why a different generative test is stronger.
Concurrency work requires repeated burst tests and evidence that mutable state is never shared across Instances.

Tests that require `/dev/kvm`, root privileges, specific filesystems, or network setup must declare those prerequisites and skip explicitly when they are absent.
A skipped KVM test is not a passing KVM test.

## Performance contributions

Performance changes must preserve correctness and isolation before comparing latency.
Provide raw samples, exact commands, commit identities, host metadata, cache state, preparation outside the timer, errors, and cleanup outcomes.
Keep cold build, cold-cache restore, warm-cache restore, prepared-resource restore, paused-Machine lease, and already-Ready lease results separate.

Claims against ComputeSDK must follow [docs/benchmark-contract.md](docs/benchmark-contract.md) exactly.
Do not modify the upstream benchmark to create a favorable boundary.
Use a provider adapter outside the upstream ComputeSDK repository when integration is required.

## Documentation

Write one complete sentence per physical line in substantial Markdown documents.
Use the canonical terms in [docs/architecture/naming.md](docs/architecture/naming.md).
Call the project `SOMA`, the public attribution `SOMA by MIOSA`, and the public repository `SOMA`.
Use lowercase `soma` for commands, package names, and source paths where required by platform convention.
Do not invent implementation details for another VMM or provider without primary public evidence.

An architectural decision record is required when a change alters any of the following:

- The public interface or protocol compatibility.
- Process or thread ownership.
- Snapshot, memory, disk, or device-state formats.
- Generation certification or compatibility rules.
- Guest authentication, Repair, or Ready semantics.
- The trusted computing base or isolation model.
- A benchmark measurement boundary.

## Pull request checklist

- The change has one clear purpose and describes the user-visible behavior.
- Tests cross the same seam as the caller and include important failures.
- Linux-specific behavior has Linux x86_64 evidence.
- Security assumptions and unsafe invariants are documented.
- Documentation and ADRs match the implementation.
- No secret, credential, private hostname, personal path, proprietary source, or unlicensed artifact is present.
- Dependency licenses and provenance are compatible with Apache License 2.0 distribution.
- Performance statements distinguish targets, measurements, and external facts.

## License

By submitting a contribution, you agree that your contribution is licensed under the repository's Apache License 2.0 terms.
You must have the right to submit every line, test vector, binary artifact, and derived work in the contribution.
Preserve upstream copyright, license, provenance, and NOTICE obligations for reused material.
