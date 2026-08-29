# SOMA portability contract

## Goal

SOMA should expose one stable sandbox-engine contract across infrastructure providers without pretending that every host, CPU, filesystem, and network has identical capabilities.
The public interface describes the required machine outcome, while a certified host adapter proves whether the local substrate can provide it.

The first supported substrate is Ubuntu 24.04 x86_64 on a KVM-capable host.
Additional environments become supported only after passing the same conformance suite.

Client portability is broader than local engine support.
The portable library and command-line tool target Linux, macOS, and Windows.
They select an explicit local backend when one satisfies the contract or communicate with a remote certified SOMA engine.
They never turn a host process or shared container daemon into an implicit sandbox fallback.

## Client and engine matrix

| Surface | Linux | macOS | Windows |
|---|---|---|---|
| Portable library and CLI | Required | Required | Required |
| Remote execution client | Planned common contract | Planned common contract | Planned common contract |
| Local engine | Ubuntu 24.04 x86_64 KVM target | Apple Silicon development adapter | Not implemented |

This matrix states design scope rather than evidence already earned.
Each release documents implemented, tested, experimental, and unsupported combinations.

The first workload contract accepts Linux OCI images.
Tags resolve to immutable manifests for an exact platform before Generation construction or execution.
Non-Linux guest operating systems require separate boot, device, agent, security, and conformance contracts and are outside `1.0.0`.

## Provider boundary

Cloud account APIs, placement, billing, quotas, instance catalogs, autoscaling, and public routing remain operator responsibilities.
SOMA begins at a prepared Linux host with authenticated launch authority and locally available certified artifacts.

This boundary allows AWS, Google Cloud, Azure, a bare-metal provider, or an on-premises cluster to use the same SOMA machine contract.
It also prevents cloud credentials and provider-specific request types from entering a guest-facing VMM process.

## Compute shapes

A `MachineSpec` expresses vCPU count, guest memory bytes, writable root capacity, and required device capabilities through validated provider-neutral values.
No public type encodes a provider SKU or a fixed catalog of sizes.

Each Generation declares exact compatibility constraints for CPU policy, page size, memory layout, device layout, guest kernel, and restore ABI.
A Generation may certify one shape or a tested family of shapes.
SOMA rejects a requested shape outside that contract rather than changing it silently.

Supporting a wide product catalog therefore means certifying reusable shape families and building missing Generations before a request, not making unsafe changes during restore.

## Storage sizes

The immutable root artifact and the requested writable capacity are separate concepts.
The root artifact contains the prepared workload state.
The Instance receives a sparse private copy-on-write head whose logical capacity may exceed the artifact's minimum when the Generation's guest repair contract supports safe growth.

Large logical disks must not require copying their full capacity during Launch.
A storage adapter must prove private mutation, sparse or thin allocation, crash behavior, quota enforcement, cleanup, and the requested logical-size range.

Persistent data volumes are independently leased attachments.
Destroying an Instance releases those leases without deleting caller-owned data.

## Networking

The host interface accepts a logical authorized network lease rather than a cloud network-interface identifier, TAP name, route command, or firewall rule.
The operator adapter realizes that lease using the local cloud and Linux network substrate.
SOMA verifies the resources transferred to the Instance and binds the repaired guest identity to the lease.

## Capability evidence

A substrate reports capabilities only after local probes and retained conformance evidence establish them.
Version strings alone are not evidence that snapshot restore, reflink isolation, seccomp, KVM ioctls, or network cleanup work correctly.

Support levels are cumulative:

1. Client conformance validates requests, limits, lifecycle semantics, idempotency, receipts, rendering, and typed capability failures on each supported client operating system.
2. KVM conformance validates machine and vCPU creation on the exact Linux architecture.
3. Isolation conformance validates namespaces, cgroups, privilege removal, seccomp, networking, storage, and cleanup.
4. Generation conformance validates boot, capture, restore, repair, execution, corruption rejection, and compatibility.
5. Burst conformance validates 100-way isolation, success, cleanup, identity uniqueness, and tail latency.

Only the fifth level supports an end-to-end performance claim for that host class and Generation.

## Extension rules

New host mechanisms remain private adapters behind cohesive SOMA policies.
A cloud adapter cannot weaken readiness, private memory, artifact immutability, identity repair, operation idempotency, or cleanup semantics.
A backend-specific configuration map is not part of the public machine contract.

ARM64, another host distribution, a different copy-on-write filesystem, or another VMM backend requires an explicit compatibility decision and conformance evidence.
SOMA will prefer a few deep, proven adapters over a broad plugin surface that moves security decisions into every caller.
An unsupported local host can remain a fully supported remote client without being listed as a supported local engine.
