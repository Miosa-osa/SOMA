# SOMA documentation

Start with the [SOMA README](../README.md) to learn what a SOMA instance is and which workloads can run on it.

## Start and run

- [Run one isolated command](../README.md#try-one-isolated-command) on a local development machine.
- [Set up a Linux KVM host](operations/server-setup.md) and run the [end-to-end check](guides/end-to-end-check.md).
- [Choose or write a Template](guides/templates.md) to reuse a workload configuration.
- [Run an interactive desktop](../workloads/desktop/README.md) with agent controls and a browser viewer.

## Security and zero trust

- [How SOMA applies zero trust](../README.md#how-soma-applies-zero-trust) explains artifact checks, per-instance isolation, authenticated readiness, and ownership checks.
- [How SOMA verifies security during development](../README.md#how-we-verify-it-during-development) covers code review rules, CI, and live KVM checks.
- [Threat model](threat-model.md) describes the trusted base, adversaries, and requirements for production use.
- [Security policy](../SECURITY.md) explains how to report a vulnerability privately.

## Integrate or operate SOMA

- [HTTP API surface](protocol/api-surface.md) lists the versioned routes and compatibility rules.
- [Agent and MCP integration](integrations/agents.md) shows how agents invoke SOMA.
- [Host and client portability](operations/deployment-portability.md) explains supported paths and host requirements.
- [Versioning and releases](versioning.md) describes release, API, and protocol compatibility.

## Understand the architecture

- [Beginner architecture guide](architecture/beginners-guide.md) follows the stack from the host to a SOMA instance.
- [Architecture diagrams](architecture/diagrams.md) trace launch, execution, and cleanup.
- [Rust module map](architecture/module-map.md) identifies which crate owns each part of the system.
- [Glossary](../GLOSSARY.md) and [naming guide](architecture/naming.md) define the shared terminology.
- [Architecture decisions](adr/) record design choices and their rationale.

## Check the evidence

- [Claim ledger](claim-ledger.md) links capability claims to the revisions and evidence that support them.
- [Benchmark contract](benchmark-contract.md) defines how performance results are measured and reported.
- [Evidence](evidence/) contains run reports and retained samples.
- [Research](research/) and [reviews](reviews/) contain design investigations and dated assessments.
