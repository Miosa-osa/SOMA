# Declaw public architecture research

This research cut was recorded on 2026-08-29 from Declaw's public documentation and public benchmark evidence.
Vendor statements are identified as claims rather than independently measured facts.

## Short answer

Declaw documents one Firecracker microVM per sandbox.
Its control plane selects a prepared root filesystem template, while its bare-metal orchestrator claims preallocated network resources and starts or restores the VM.
An in-guest `envd` process exposes command and filesystem operations over a private control path.
Node, Python, agent SDKs, and similar tools come from the selected template rather than from Firecracker or the VMM.

## Documented topology

```text
SDK or CLI
   |
   v
control plane
   |
   v
bare-metal orchestrator
   |-- template and snapshot cache
   |-- per-sandbox network namespace, veth, and TAP
   |-- host-side security proxy
   |
   v
Firecracker microVM
   |-- base ext4 root
   |-- private writable overlay
   `-- envd as guest control service
```

This is Declaw's disclosed architecture, not SOMA's implementation.

## Templates and workload contents

Declaw defines a template as the root filesystem used to boot a sandbox.
Its documented default is `base`, described as Ubuntu 22.04 with Git, curl, wget, jq, and build-essential.
It also documents `python`, `node`, and `ai-agent` built-ins.
Its `node` template includes Node.js 20 LTS, npm, TypeScript, and Yarn.
Its custom template API accepts Dockerfile-like build content, copied files, an alias, and default CPU and memory values.

Declaw's Commands page separately says the default sandbox template includes Python, Node.js, Go, and shell utilities.
That statement conflicts with the Templates page's explicit `base` default and should not be treated as resolved without clarification from Declaw.

Completed custom templates are documented as immutable.
A changed recipe requires another completed template rather than mutating the filesystem used by existing sandboxes.
Builds are asynchronous and expose pending, building, done, and error states.

The transferable product insight is that users need a friendly preparation recipe, but sandbox Launch should reference already prepared immutable content.
SOMA makes that split explicit as Template, Template Lock, Generation, and Instance.

## Filesystem and storage

Declaw documents a shared read-only base ext4 image at `/opt/declaw/rootfs/rootfs.ext4` and a per-sandbox writable overlay at `/opt/declaw/run/<sandbox_id>/overlay.ext4`.
The guest sees the merged result through OverlayFS.
The per-sandbox host directory also contains Firecracker control and log files and may contain proxy certificate material.

The useful pattern is immutable shared base data plus private writable state.
SOMA's selected filesystem contract differs, using deterministic EROFS for immutable content and private ext4 for the writable OverlayFS upper layer.

## Network design

Declaw documents one Linux network namespace, veth pair, and TAP device per sandbox.
It states that a host-side Layer 7 TCP proxy and kernel filtering enforce outbound policy.
Its public sandbox API supports allow and deny rules for domains, IP addresses, and CIDRs.
It documents unrestricted egress as the general sandbox default, while its MCP wrapper changes that user experience to deny-all unless domains are explicitly allowed.

Declaw documents automatic blocking of the IPv4 cloud metadata address `169.254.169.254`.
It also documents that domain filtering applies to TCP ports 80 and 443 and does not cover UDP or QUIC.
Those limitations are important because a domain policy must not imply protocol coverage it does not enforce.

Declaw states that it preallocates a pool of 256 network slots and replenishes the pool in the background.
That is a preparation-pool size, not evidence that a host can run only or at least 256 resident sandboxes.

## Environment and secrets

Declaw documents ordinary sandbox-level and command-level environment variables.
It also distinguishes protected environment values from a credential-vault flow.
In its documented vault flow, the guest receives a placeholder and a host-side egress proxy injects the real credential for an approved destination.

The transferable distinction is valuable:

- Some programs genuinely require a secret inside their process environment or a file.
- Some programs only need authenticated outbound requests, so a host-side mediator can keep the secret outside the guest.

SOMA must represent those delivery modes explicitly and must never place a reusable secret in a Template, Template Lock, Generation, snapshot, log, or receipt.

## Lifecycle and commands

Declaw documents sandbox timeout, kill or pause behavior on timeout, and optional automatic resume.
It documents blocking commands, streamed commands, background commands, standard input, working directory, user selection, per-command environment, and command timeout.

These are product primitives above the VMM.
They do not explain how a VM boots, but they define the guest-control and lifecycle surface users expect from an agent sandbox.

## MCP and coding-agent use

Declaw's documented `declaw mcp -- <command>` wrapper places a stdio MCP server inside a sandbox.
It supports an image template, selected environment forwarding, file upload, domain allowlisting, stdio bridging, and automatic cleanup when the client disconnects.

The strongest interface lesson is the prefix pattern.
A local client can retain the standard stdio MCP contract while a sandbox wrapper handles preparation, policy, upload, transport, and cleanup.
SOMA can provide the same class of interoperability without coupling its core to a particular coding agent.

## Resource and performance claims

Declaw documents a default of 1 vCPU and 256 MB of memory, with ranges of 1 to 8 vCPUs and 128 MB to 8 GB of memory.
It documents a fixed 20 GB writable overlay.
Declaw claims approximately 125 ms for cold `CreateVM()` through guest readiness and approximately 30 ms for snapshot restore.

ComputeSDK independently recorded 476.53 ms median TTI, 573.91 ms p95, 575.96 ms p99, and 100 percent success on 2026-08-28.
The vendor and ComputeSDK numbers use different measurement boundaries and must not be treated as contradictory measurements of the same interval.

Declaw documents a maximum of 1024 concurrent sandbox creation operations and an execution semaphore sized at four times host CPU count during creation.
These are concurrency guards, not proof of 1024 simultaneously resident VMs.

## What remains unknown

The reviewed public pages do not establish:

- The complete snapshot identity, entropy, time, socket, and credential repair protocol.
- The exact guest authentication and session-key rotation mechanism.
- Reproducible raw latency samples for the vendor boot claims.
- The full bypass-resistance proof for proxy, DNS, IPv6, raw sockets, and non-TCP traffic.
- Host density under declared workload and dirty-memory distributions.
- Snapshot compatibility rules across CPUs, kernels, VMM releases, and device state.
- The complete isolation threat model and independent audit evidence.
- Which documented default workload is current, because the Templates and Commands pages disagree.

SOMA must not fill these gaps with assumptions.

## Sources

- [Declaw architecture overview](https://docs.declaw.ai/architecture/overview)
- [Declaw Firecracker architecture](https://docs.declaw.ai/architecture/firecracker)
- [Declaw templates](https://docs.declaw.ai/features/templates)
- [Declaw networking](https://docs.declaw.ai/features/networking)
- [Declaw network policies](https://docs.declaw.ai/security/network-policies)
- [Declaw environment secrets](https://docs.declaw.ai/security/env-secrets)
- [Declaw credential vault](https://docs.declaw.ai/security/credential-vault)
- [Declaw sandboxes](https://docs.declaw.ai/features/sandboxes)
- [Declaw commands](https://docs.declaw.ai/features/commands)
- [Declaw MCP sandboxing](https://docs.declaw.ai/cli/mcp)
- [ComputeSDK benchmarks](https://github.com/computesdk/benchmarks)
