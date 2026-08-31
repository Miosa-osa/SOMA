# SOMA naming

## Canonical identity

The project name is **SOMA**.
The expansion is **Secure Optimized Machine Architecture**.
The full attribution form is **SOMA by MIOSA**.
The repository name is `SOMA`, and its canonical public path is `Miosa-osa/SOMA`.

Always write the project name as `SOMA` in prose.
Do not introduce `Soma`, `SomaVM`, `SomaOS`, `SOMA Sandbox`, or a second expansion.

SOMA names a provider-neutral sandbox engine and machine execution architecture, not one provider's customer-facing sandbox product.
An operator may build a sandbox, function, computer, runner, or agent environment on top of SOMA without changing SOMA's category.

## Software names

- Rust crates use the `soma-` prefix.
- The per-machine VMM binary is `soma-vmm`.
- The command-line entry point is reserved as `soma`.
- The versioned local protocol identifier is `soma.vmm.v1`.
- A long-lived daemon name is not reserved because the first topology does not require a daemon.

## Domain language

| Term | Exact meaning | Do not substitute |
|---|---|---|
| Template | A reusable user-authored recipe that is compiled and certified into a Generation | Generation, snapshot, Instance |
| Generation | A certified immutable set of kernel, root filesystem, memory, machine state, guest-agent, device-layout, and compatibility metadata | Image, template, snapshot |
| Artifact | One immutable content-addressed file within a Generation | Asset, blob |
| Snapshot | The captured memory and machine state within a Generation | Generation |
| Machine | One hardware-virtualized guest owned by one `soma-vmm` process | Container, process |
| Instance | One globally unique concrete Machine lifetime owned by one `soma-vmm` process | Stable resource, sandbox |
| Launch | The atomic request that creates or restores an Instance and proves readiness | Boot, spawn |
| Milestone | One monotonic observation within Launch | Status, log line |
| Ready | Clone repair is complete and reported under the Instance's own authenticated session | Running, resumed, connected |
| Repair | Replacement of cloned identity, entropy, time, network, and transport state | Initialization |
| Receipt | The immutable result that binds a request, Instance, Generation, milestones, and outcome | Response |

The word `sandbox` remains a product-level term owned by callers.
The VMM does not decide tenant plans, billing, placement, pooling policy, or public resource semantics.
An operator may retain a stable resource identity across multiple Instance lifetimes, but that identity remains outside the per-Machine VMM interface.
An `InstanceId` must never be reused for another Machine lifetime.

The authoritative product glossary is [CONTEXT.md](../../CONTEXT.md).
The beginner-first relationship between these terms is explained in [From hardware to an agent sandbox](beginners-guide.md).

## Name rationale

The Greek word `soma` means body.
Within the MIOSA family, Optimal and OSA provide intelligence while SOMA provides the isolated machine body that performs work.
The acronym also states the engineering contract directly: security, optimization, machines, and architecture.

## Naming lessons from other projects

A short distinctive root plus one precise category sentence is more durable than a generic `AgentOS`, `FastVM`, or `Sandbox` compound.
Casing is part of identity and must not drift across repositories, packages, documentation, and binaries.
The project must never imply that it owns a technical layer it merely integrates.
SOMA may use external crates and may begin by comparing against existing VMMs, but the repository describes those relationships precisely.
