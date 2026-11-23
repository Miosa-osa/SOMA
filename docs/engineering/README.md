# Engineering docs

Understand the implementation, work on the code, and trace decisions and claims to their evidence.

## Read or change the code

- [Beginner architecture guide](architecture/beginners-guide.md) follows the stack from the host to an instance.
- [How one SOMA instance works](architecture/how-it-works.md) traces the original KVM bring-up and boot path.
- [Architecture diagrams](architecture/diagrams.md) trace launch, execution, and cleanup.
- [Rust module map](architecture/module-map.md) identifies the owning crate for each part of the system.
- [Protocol specification](protocol/v1.md) defines the local VMM wire contract.
- [Glossary](../../GLOSSARY.md) and [naming guide](architecture/naming.md) define the shared terminology.
- [Architecture decisions](adr/) record design choices and their rationale.
- [Contributing](../../CONTRIBUTING.md) explains the development and review workflow.

## Security, tests, and evidence

- [Management-plane security](management-plane-security.md) maps identity and access enforcement to the platform and runner.
- [Threat model](threat-model.md) describes the trusted base, adversaries, and requirements for production use.
- [How SOMA verifies security during development](../../README.md#how-we-verify-it-during-development) covers review, CI, and live KVM checks.
- [Dependency policy](dependency-policy.md) explains how dependencies are selected and audited.
- [Claim ledger](claim-ledger.md) links capability claims to the revisions and evidence that support them.
- [Benchmark contract](benchmark-contract.md) defines how performance results are measured and reported.
- [Security policy](../../SECURITY.md) explains how to report a vulnerability privately.

## Design history and reference material

- [Evidence reports and samples](evidence/) record what was measured at specific revisions.
- [Runtime footprint measurements](evidence/2026-11-22-runtime-footprint.md) record artifact sizes and a desktop process sample.
- [Research](research/) and [reviews](reviews/) contain design investigations and dated assessments.
- [August 2026 handover](HANDOVER.md) records the state of the early KVM implementation.

[Documentation home](../README.md)
