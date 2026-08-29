# Deployment portability

## Contract

SOMA separates where a caller runs from where a sandbox engine runs.
The portable library, CLI, and MCP server are caller adapters.
A local or remote engine is a capability-gated execution substrate.

This separation lets an operator place SOMA workers on suitable hosts in a public cloud, a private cloud, a colocation facility, or an on-premises cluster without putting provider names into the Machine interface.
It does not mean that every compute product can host a virtual machine monitor.

## Support levels

| Level | Meaning |
| --- | --- |
| Certified engine host | The exact host class, operating system, kernel, CPU model, storage mode, network mode, and SOMA build passed conformance, isolation, cleanup, and retained performance gates. |
| Candidate engine host | The substrate exposes the required capabilities, but the exact combination has not passed the complete SOMA certification matrix. |
| Client-only location | The environment can invoke a remote SOMA engine but cannot satisfy the local engine contract. |
| Unsupported | Neither a certified local engine nor an explicitly configured remote engine is available, so SOMA fails closed. |

Support attaches to an exact host profile rather than a cloud logo.
Changing the instance family, CPU generation, kernel, filesystem, nested-virtualization layer, or network implementation can change the result and requires a new certification record.

## Initial Linux engine profile

The first production profile targets Ubuntu 24.04 on x86_64 hosts with KVM.
An engine candidate must provide all of the following before a workload is admitted:

- A compatible KVM interface and the required CPU virtualization features.
- Sufficient CPU, memory, and storage capacity for the requested Machine shape.
- cgroup v2 and the required namespace, seccomp, process, and resource-accounting controls.
- A supported private networking path with enforceable denied and allowed policies.
- A supported private writable-root mechanism and verifiable cleanup.
- A filesystem and kernel combination accepted by the selected Generation and snapshot profile.
- Stable monotonic timing and enough reserved capacity for the declared performance class.
- Permission to run SOMA's constrained VMM processes without silently weakening their jail.

`soma doctor --strict` is the local preflight interface.
A passing preflight means the host exposes the prerequisites checked by that SOMA release.
It is not a substitute for the complete target-host conformance and security suite.

## Deployment locations

| Location | Intended role | Current status |
| --- | --- | --- |
| Ubuntu 24.04 x86_64 bare metal | Production engine candidate | Initial stable target, with the real KVM lifecycle still under construction. |
| On-premises or colocated Linux hosts | Production engine candidate | Provider-neutral when the exact host profile passes certification. |
| AWS EC2 bare-metal hosts | Production engine candidate | AWS documents hardware access intended for virtualization workloads, but SOMA has not certified an EC2 profile yet. |
| Supported AWS EC2 nested-virtualization instances | Engine candidate | AWS documents nested KVM on selected instance types and recommends bare metal for strict latency requirements. |
| Google Compute Engine with nested virtualization | Engine candidate | Google documents KVM-backed nested guests, but SOMA has not certified a Compute Engine profile yet. |
| Other public-cloud Linux machines | Engine candidate only when capabilities are explicit | A generic VM name is not evidence that KVM, isolation, or the latency contract is available. |
| Apple Silicon macOS 26 | Development engine | Real VM-per-OCI lifecycle through Apple Container 1.3, without Linux KVM certification. |
| AWS Lambda and similar managed function runtimes | Client-only location | A function can call a remote SOMA fleet, but the managed function environment is not a SOMA KVM host profile. |
| Windows and Intel macOS | Portable client | No local SOMA isolation engine is certified in the alpha. |

AWS maintains the current nested-virtualization instance list in its [EC2 nested virtualization guide](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/amazon-ec2-nested-virtualization.html).
AWS describes bare-metal Nitro instances as suitable for workloads requiring low-level hardware features in its [Nitro instance guide](https://docs.aws.amazon.com/ec2/latest/instancetypes/ec2-nitro-instances.html).
Google documents its nested KVM procedure in the [Compute Engine nested virtualization guide](https://cloud.google.com/compute/docs/instances/nested-virtualization/creating-nested-vms).
These provider documents establish candidate capability only.
SOMA support still requires its own retained conformance evidence.

## Portable deployment shape

A portable SOMA deployment has three independently replaceable layers:

1. Callers use the same bounded run and managed-lifecycle use cases through the Rust facade, CLI, or MCP server.
2. An operator-owned control plane authenticates requests, chooses a certified host profile, and transfers one operation to one engine host.
3. Each engine host uses a target adapter to launch one VMM process for one Machine and returns the same validated receipt schema.

Cloud placement, autoscaling, billing, tenant policy, DNS, and public authentication remain outside this repository.
The SOMA repository owns the execution contract, host qualification, local lifecycle, VMM, and evidence required to keep those external control planes interchangeable.

## Remote use

An authenticated remote adapter is an accepted design but is not implemented in this alpha.
When it lands, it must preserve the same request fingerprint, operation identity, output bounds, lifecycle semantics, and receipt validation as a local engine.
It must never convert an unsupported local request into a weaker host-process or container-only execution path.

This is how a Lambda function, Windows workstation, CI controller, or agent platform can use SOMA without pretending that its own process environment is a KVM host.

## Certification evidence

Every published host profile must retain:

- Provider and region when applicable.
- Bare-metal or nested-virtualization status.
- Instance family or hardware model and CPU identity.
- Operating system, kernel, KVM interface, filesystem, and network mode.
- SOMA source revision, release, and exact Generation identity.
- Security and cleanup conformance results.
- Raw one-shot, managed-lifecycle, failure, and burst samples.
- Cache state, preparation class, concurrency, shape, and timer boundary.
- Known unsupported capabilities and any unavailable evidence dimensions.

A deployment guide may claim only the support level demonstrated by that evidence.
