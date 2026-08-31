# The provider contract, and what SOMA does not implement

This is a gap analysis against the interface SOMA must satisfy to be a sandbox provider, taken from the ComputeSDK provider surface and from the published architecture of the providers already on that list.
It records what is missing rather than what works, because the working parts are already in [the claim ledger](../claim-ledger.md).

Vendor statements below are claims from public documentation, not independently measured facts.

## Why this contract and not another

ComputeSDK defines one interface that every provider implements, and its provider list already includes MIOSA alongside E2B, Modal, Vercel, Daytona, Runloop, Superserve, Isorun, Declaw, Microsandbox, Sandbox0, and roughly twenty more.
That makes the interface a specification SOMA can be measured against rather than a competitor feature list to copy selectively.
A provider that implements only part of it is not a provider that is merely slower.

## The contract

| Operation | SOMA today |
| --- | --- |
| `sandbox.create()` | Implemented as one-shot `run` |
| `sandbox.destroy()` | Implemented, with proven cleanup |
| `sandbox.getById()` | **Missing.** No Instance survives the process that launched it |
| `sandbox.list()` | **Missing.** Nothing enumerates live Instances |
| `runCommand()` | One bounded command per sandbox, stdout, stderr, exit status |
| `filesystem.readFile()` | **Missing** |
| `filesystem.writeFile()` | **Missing** |
| `filesystem.mkdir()` | **Missing** |
| `filesystem.readdir()` | **Missing** |
| `filesystem.exists()` | **Missing** |
| `filesystem.remove()` | **Missing** |
| Interactive PTY terminal | **Missing** |

The guest control protocol is the reason the second half of that table is empty.
It carries eight frame kinds: `Prepare`, `Execute`, and `Shutdown` from the host, and `RepairComplete`, `Stdout`, `Stderr`, `Terminal`, and `ShutdownAck` from the guest, where `Terminal` is the exit status of a command rather than a pseudo-terminal.
There is no frame that reads a file, writes one, lists a directory, or attaches to a terminal, so no host API can offer those however it is written.

`getById` and `list` are missing for a different reason, already decided rather than undecided: the KVM Backend owns at most one live Machine inside the calling process, so an Instance cannot outlive the command that created it.
[ADR 0031](../adr/0031-persistent-host-runtime-ownership.md) is the accepted answer and is not implemented.

## What the category treats as table stakes

A 2026 survey of coding-agent sandboxes lists process isolation, filesystem sandboxing, network denied by default, credential protection that keeps tokens out of the agent's reach, sub-90 millisecond startup, and git worktree support for parallel agents.

SOMA has process isolation and filesystem isolation, and its measured sequential time to first command is 65.5 ms, which is inside that startup band.
It has no filesystem API, no credential mechanism, and no network at all.

## Network

The largest single gap is not a missing subsystem. It is an unwired one.

`soma-netd` is about 7,800 lines and implements network namespaces, netlink, nftables rulesets, address management, DNS planning, ingress, activation, reconciliation, a daemon, and a transfer protocol.
`soma-kvm` implements `TapBackend` over a preopened non-blocking TAP descriptor.
The KVM Backend uses neither: `link_down_network` hands the guest a placeholder with a fixed address and the link down, so no packet leaves any sandbox.

Everything a coding agent does needs that link up. Installing a package, cloning a repository, pulling a container image, and calling a model API are all egress.
A sandbox without egress can run a command against content baked into its Generation and nothing else.

## Credentials

Declaw's published design distinguishes two delivery modes, and the distinction is the useful part rather than the implementation.
Some programs need a secret in their own process environment or a file. Others only need authenticated outbound requests, which a host-side mediator can perform without the secret ever entering the guest.

SOMA's Template schema already carries `secrets` with a file mode and a default of owner-read-only, so the authoring side of the first mode is designed.
Nothing delivers a secret to a running Instance, and no host-side mediator exists, so the second mode has no implementation and the first has no runtime.

## Templates

`soma-template` is about 6,400 lines and parses a TOML document with a name, workload, modules, command, resources, network, lifecycle, environment, and secrets, then composes and resolves it into a canonical Template Lock.
The repository contains no example Template document, and the ledger records a Generation built from a Template Lock as designed rather than implemented.

E2B has moved its authoring surface from a Dockerfile to a chained builder, with `fromImage`, `copy`, `runCmd`, `aptInstall`, `pipInstall`, `gitClone`, `setEnvs`, `setStartCmd`, and `setReadyCmd`, and keeps `fromDockerfile` for migration.
The relevant lesson is not the syntax. It is that the authoring surface is small, ordered, and separate from the immutable artifact it produces, which is the split SOMA already names Template, Template Lock, Generation, and Instance.

## What this means for order of work

Networking comes first because it gates the value of everything else, and because it is integration of two implemented subsystems rather than new work.
The guest protocol comes second, because the filesystem and terminal halves of the contract cannot be written above a protocol that has no frames for them.
A persistent Host Runtime comes third, because `getById` and `list` are not API surface but ownership, and because the prepared worker path that removes the measured 48 ms of machine construction needs the same runtime.
Credential delivery and an authorable Template follow, and neither can be proved without the three above.

## What this document is not

- It is not a measurement. The only figures in it are the retained ones already published in the evidence records.
- It is not a commitment to implement the contract in this order, or at all.
- It does not evaluate any competitor's implementation, only its published description.
