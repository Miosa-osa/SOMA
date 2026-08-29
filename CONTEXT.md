# SOMA domain language

This glossary defines the product concepts used across SOMA.
It intentionally excludes implementation details.

## Sandbox

A sandbox is the product-level isolated environment in which a workload runs.
It has a lifecycle, resource limits, an isolation class, and execution evidence.

## Template

A Template is a reusable user-authored recipe for producing a Generation.
It may select an OCI image, Machine shape, startup behavior, network policy, and other preparation inputs.
A Template is not a running sandbox and is not itself a snapshot.

## Generation

A Generation is a certified immutable machine state from which Instances can be launched repeatedly.
Changing any input produces a different Generation identity.

## Snapshot

A Snapshot is captured memory and virtual-machine state stored as part of a Generation.
A Snapshot is not a complete Generation because it does not contain every workload artifact, compatibility rule, or identity-repair requirement.

## Machine

A Machine is one hardware-virtualized guest environment.
It owns guest memory, virtual CPUs, devices, storage attachments, and network attachments.

## Instance

An Instance is one globally unique lifetime of a Machine.
Each launch creates a fresh Instance identity even when many launches use the same Generation.

## Launch

Launch is the atomic operation that realizes a Generation as an Instance and proves the Instance is Ready.

## Ready

Ready means clone repair is complete and an authenticated command has succeeded inside the guest.
A process existing or a virtual CPU running is not sufficient evidence of Ready.

## Repair

Repair replaces cloned identity, entropy, time, network, and transport state before an Instance becomes Ready.

## Backend

A Backend is an implementation of the sandbox lifecycle for one execution substrate.
Different Backends may provide different isolation and performance properties while preserving the public lifecycle semantics.

## Receipt

A Receipt is immutable evidence that binds an operation to its Instance, Generation, effective isolation, milestones, result, and cleanup state.

## Machine shape

A Machine shape is the requested vCPU, memory, and writable-storage capacity of one Instance.
It is distinct from the physical Host capacity and from a provider's commercial size name.

## vCPU

A vCPU is one virtual processor visible to a guest Machine.
It is scheduled onto a Host hardware thread and does not imply permanent ownership of one physical CPU core.

## Host

A Host is the physical or virtual machine that supplies CPU time, memory, storage, networking, and process ownership for a set of independent Instances.

## Workload runtime

A Workload runtime is an optional language or application runtime, such as Node.js or Python, included by the selected workload.
It is not part of SOMA's virtualization foundation and is not required when the workload is a directly executable native program.

## Guest agent

The Guest agent is the trusted SOMA component inside a Machine that authenticates the Instance, completes Repair, executes bounded commands, and reports evidence.
It is distinct from the caller's external agent and from the user's workload program.
