# Product and usage docs

Learn what SOMA does, run workloads, and integrate it with your platform.

## Learn what SOMA does

- [What you can run](../../README.md#what-you-can-run) explains how agent sandboxes, desktops, and services use SOMA instances.
- [Size and memory footprint](reference/footprint.md) breaks down download sizes, binaries, and workload memory.
- [Runtime comparison](reference/runtime-comparison.md) compares SOMA with Firecracker and other isolation runtimes.
- [How an instance starts](../../README.md#how-an-instance-starts) defines Templates, Generations, and instance lifetimes.
- [How SOMA applies zero trust](../../README.md#how-soma-applies-zero-trust) explains artifact verification, per-instance isolation, and authenticated readiness.

## Run a workload

- [Run one isolated command](../../README.md#try-one-isolated-command) on a local development machine.
- [Choose or write a Template](guides/templates.md) to reuse a workload configuration.
- [Run an interactive desktop](../../workloads/desktop/README.md) with agent controls and a browser viewer.

## Integrate or operate SOMA

- [HTTP API surface](reference/api-surface.md) lists versioned routes and compatibility rules.
- [Agent and MCP integration](integrations/agents.md) shows how agents invoke SOMA.
- [Set up a Linux KVM host](operations/server-setup.md) and run the [end-to-end check](guides/end-to-end-check.md).
- [Host and client portability](operations/deployment-portability.md) explains supported paths and host requirements.
- [Versioning and releases](reference/versioning.md) describes release, API, and protocol compatibility.

[Documentation home](../README.md)
