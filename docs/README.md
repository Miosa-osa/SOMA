# SOMA documentation

Choose [product and usage docs](#product-and-usage-docs) to learn what SOMA does and use it.
Choose [engineering docs](#engineering-docs) to understand the implementation, contribute, or evaluate evidence.

## Product and usage docs

### Learn what SOMA does

- [What you can run](../README.md#what-you-can-run) explains how agent sandboxes, desktops, and services use SOMA instances.
- [How an instance starts](../README.md#how-an-instance-starts) defines Templates, Generations, and instance lifetimes.
- [How SOMA applies zero trust](../README.md#how-soma-applies-zero-trust) explains artifact verification, per-instance isolation, and authenticated readiness.

### Run a workload

- [Run one isolated command](../README.md#try-one-isolated-command) on a local development machine.
- [Choose or write a Template](guides/templates.md) to reuse a workload configuration.
- [Run an interactive desktop](../workloads/desktop/README.md) with agent controls and a browser viewer.

### Integrate or operate SOMA

- [HTTP API surface](protocol/api-surface.md) lists versioned routes and compatibility rules.
- [Agent and MCP integration](integrations/agents.md) shows how agents invoke SOMA.
- [Set up a Linux KVM host](operations/server-setup.md) and run the [end-to-end check](guides/end-to-end-check.md).
- [Host and client portability](operations/deployment-portability.md) explains supported paths and host requirements.
- [Versioning and releases](versioning.md) describes release, API, and protocol compatibility.

## Engineering docs

### Read or change the code

- [Beginner architecture guide](architecture/beginners-guide.md) follows the stack from the host to an instance.
- [Architecture diagrams](architecture/diagrams.md) trace launch, execution, and cleanup.
- [Rust module map](architecture/module-map.md) identifies the owning crate for each part of the system.
- [Protocol specification](protocol/v1.md) defines the local VMM wire contract.
- [Glossary](../GLOSSARY.md) and [naming guide](architecture/naming.md) define the shared terminology.
- [Architecture decisions](adr/) record design choices and their rationale.
- [Contributing](../CONTRIBUTING.md) explains the development and review workflow.

### Security, tests, and evidence

- [Threat model](threat-model.md) describes the trusted base, adversaries, and requirements for production use.
- [How SOMA verifies security during development](../README.md#how-we-verify-it-during-development) covers review, CI, and live KVM checks.
- [Dependency policy](dependency-policy.md) explains how dependencies are selected and audited.
- [Claim ledger](claim-ledger.md) links capability claims to the revisions and evidence that support them.
- [Benchmark contract](benchmark-contract.md) defines how performance results are measured and reported.
- [Security policy](../SECURITY.md) explains how to report a vulnerability privately.

### Design history and reference material

- [Evidence reports and samples](evidence/) record what was measured at specific revisions.
- [Research](research/) and [reviews](reviews/) contain design investigations and dated assessments.
- [August 2026 handover](HANDOVER.md) records the state of the early KVM implementation.
