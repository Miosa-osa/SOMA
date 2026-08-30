# Competitive Research Ledger

Research cut: 2026-08-28 UTC.

This ledger records externally verifiable architecture, isolation, and performance evidence that may inform SOMA.

It is research material, not an endorsement, a purchasing guide, or a claim that unlike measurements are directly comparable.

No private tests, credentials, customer information, or non-public disclosures are included.

## Evidence rules

- **Verified design disclosure** means a first-party document or source tree states the design, but it does not mean that an independent security audit verified the implementation.
- **Independent observation** means a public third party measured the result rather than the provider being profiled, but it does not guarantee impartiality or eliminate methodology limits.
- **Vendor claim** means the provider published the statement about its own service.
- **Project benchmark** means an open-source project published a reproducible measurement of its own software.
- **Unknown** means the reviewed primary sources did not disclose enough evidence to make the claim.
- Performance numbers retain their original measurement boundary because VM boot, snapshot restore, API create, and time to first successful command are different metrics.
- A failed or missing run is recorded as unavailable and is never converted into zero latency.

## ComputeSDK benchmark snapshot

The independent observations below come from the public ComputeSDK burst time-to-interactive run timestamped 2026-08-28T13:48:39.760Z.

The run used Linux x64, 100 concurrent creates per provider, a 120 second timeout, and measured from the create request through the first successful command according to the repository methodology.

The table uses the benchmark's own composite rank, median, p95, p99, and success rate without normalizing different provider products or isolation modes.

| Rank | Provider | Composite | Median TTI | p95 TTI | p99 TTI | Success |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | Isorun | 99.30 | 63.90 ms | 79.35 ms | 79.86 ms | 100% |
| 2 | CreateOS | 97.84 | 205.27 ms | 231.71 ms | 232.73 ms | 100% |
| 3 | Archil | 96.98 | 262.85 ms | 353.08 ms | 371.86 ms | 100% |
| 4 | Arker | 96.72 | 310.93 ms | 349.67 ms | 358.76 ms | 100% |
| 5 | Declaw | 94.84 | 476.53 ms | 573.91 ms | 575.96 ms | 100% |
| 6 | Blaxel | 94.50 | 516.24 ms | 585.30 ms | 626.31 ms | 100% |
| 7 | Vercel | 93.78 | 536.74 ms | 704.07 ms | 826.20 ms | 100% |
| 8 | Superserve | 93.74 | 486.54 ms | 814.03 ms | 868.24 ms | 100% |
| 10 | Modal | 91.09 | 849.24 ms | 949.48 ms | 959.87 ms | 100% |
| 11 | Google Cloud Run | 91.05 | 682.17 ms | 1,170.27 ms | 1,285.20 ms | 100% |
| 12 | Runloop | 89.13 | 1,058.37 ms | 1,128.52 ms | 1,131.07 ms | 100% |
| 13 | E2B | 88.16 | 1,085.47 ms | 1,306.85 ms | 1,374.73 ms | 100% |
| 14 | Daytona | 87.45 | 326.21 ms | 481.78 ms | 492.68 ms | 91% |
| 18 | Tensorlake | 81.85 | 1,404.60 ms | 2,431.01 ms | 2,431.96 ms | 100% |
| 19 | Tenki | 81.29 | 1,674.37 ms | 2,162.74 ms | 2,171.38 ms | 100% |
| 20 | Sail | 72.01 | 2,683.83 ms | 2,960.78 ms | 2,988.61 ms | 100% |
| 21 | Cloudflare | 40.57 | 5,255.33 ms | 6,714.63 ms | 7,408.34 ms | 100% |
| 22 | Upstash | 36.51 | 5,272.94 ms | 7,935.29 ms | 8,007.29 ms | 100% |
| 24 | Run Cloud | 0.00 | 11,808.46 ms | 22,603.99 ms | 26,349.86 ms | 100% |
| Not ranked | Lightning AI | 0.00 | unavailable | unavailable | unavailable | 0% |

Absolute rank gaps are other providers in the public result that are outside this requested ledger, including MIOSA at rank 9.

The current authoritative identities at ranks 11 through 13 are Google Cloud Run, Runloop, and E2B.

This establishes the current 2026-08-28 ranking, but it does not prove that those three names occupied missing rows 11 through 13 in an undated screenshot.

Reconstructing that screenshot-specific gap requires its capture date or the matching recorded result, so the screenshot-specific identities remain unknown.

Lightning AI had no successful samples in the 2026-08-28 run, while the last successful public result reviewed here was 2026-08-21 with 432.92 ms median, 486.58 ms p95, 502.99 ms p99, and 100% success.

Primary benchmark sources are the [immutable 2026-08-28 raw result](https://github.com/computesdk/benchmarks/blob/bb60a6466f04278e92030f6d740fa3dce750d29b/results/burst_tti/2026-08-28.json), [methodology at the same revision](https://github.com/computesdk/benchmarks/blob/bb60a6466f04278e92030f6d740fa3dce750d29b/METHODOLOGY.md), [benchmark repository](https://github.com/computesdk/benchmarks), and [immutable 2026-08-21 result for Lightning AI](https://github.com/computesdk/benchmarks/blob/d234071386ad4859ff035927116511d563c096e0/results/burst_tti/2026-08-21.json).

## Hosted provider profiles

### Isorun

- **Architecture and isolation:** **Verified design disclosure.** Isorun documents isolated Linux microVMs with a separate kernel, hardware isolation, full-state fork, hibernate, resume, snapshot, and restore operations.
- **Default shape:** **Verified design disclosure.** The quickstart states that a sandbox defaults to 1 vCPU, 1 GiB of memory, and a 4 GiB disk.
- **Performance:** **Independent observation, 2026-08-28.** ComputeSDK recorded 63.90 ms median TTI, 79.35 ms p95, 79.86 ms p99, and 100% success.
- **Published claim:** **Vendor claim.** The API reference contains an illustrative create response with `create_ms: 9`, and the public site states that sandboxes are "ready in 10ms".
- **Server-side create:** **Independent observation, 2026-08-30.** A direct measurement of the service's own reported `create_ms` over 250 successful sandboxes recorded 22 ms p50 and 27 ms p99 for sequential `node:22` creates, 73 ms p50 and about 208 ms p99 at concurrency 100 across two agreeing runs, and a fastest observed create of 19 ms. No sample reached 10 ms or 15 ms. A create from an unprepared image reported 52 ms while the caller waited 4,808 ms, so the reported quantity excludes image preparation. Details and limits are in [the retained measurement](docs/evidence/2026-08-30-isorun-create-latency.md).
- **Published scale claim:** **Vendor claim.** Isorun's builder reports creating 100,000 concurrent sandboxes in 24 seconds in the ComputeSDK Scale Invitational, but the reviewed material does not expose raw per-sandbox samples or a directly comparable Burst TTI boundary for that event.
- **Best insight for SOMA:** Make fork, hibernate, snapshot, and restore explicit lifecycle capabilities instead of hiding them behind a generic start operation.
- **Second insight for SOMA:** The measured degradation from 22 ms sequential to 73 ms at concurrency 100 shows that a leading service's advantage is not flat under burst, so SOMA's own admitted result must report every concurrency rung rather than a single best case.
- **Pitfall or unknown:** The reviewed pages do not identify the VMM, jail boundary, restore identity-repair contract, or an independent isolation audit.
- **Primary sources:** [Isorun documentation](https://docs.isorun.ai/), [quickstart](https://docs.isorun.ai/getting-started/quickstart), [API reference](https://docs.isorun.ai/api-reference), and [builder launch account](https://www.linkedin.com/pulse/from-sarajevo-internet-caf%C3%A9s-100000-agent-sandboxes-why-beganovi%C4%87-qrjne).

### CreateOS

- **Architecture and isolation:** **Verified design disclosure.** CreateOS documents a Firecracker microVM per workload, a dedicated guest kernel, a read-only image plus writable state, outbound-only network posture, eBPF enforcement, private sandbox networking, snapshot pause, and fork.
- **Performance:** **Independent observation, 2026-08-28.** ComputeSDK recorded 205.27 ms median TTI, 231.71 ms p95, 232.73 ms p99, and 100% success.
- **Published claim:** **Vendor claim.** The product page describes memory, vCPU, and device state preservation during pause but does not publish a directly comparable public TTI number reviewed here.
- **Best insight for SOMA:** Treat pause, fork, durable storage attachment, and private networking as separate adapter capabilities with explicit state transitions.
- **Pitfall or unknown:** Full-state clones must repair entropy, instance identity, host bindings, and network identity, while the reviewed sources do not provide independent security validation.
- **Primary sources:** [CreateOS Sandbox](https://createos.sh/products/sandbox) and [CreateOS CLI](https://github.com/NodeOps-app/createos-cli).

### Archil

- **Architecture and isolation:** **Verified design disclosure.** Archil describes a dedicated container with assigned CPU and memory for each execution and a persistent, multi-attach filesystem backed by object storage.
- **Performance:** **Independent observation, 2026-08-28.** ComputeSDK recorded 262.85 ms median TTI, 353.08 ms p95, 371.86 ms p99, and 100% success.
- **Published claim:** **Vendor claim.** Archil presents filesystem identity as the durable sandbox abstraction and intentionally hides compute lifecycle from callers.
- **Best insight for SOMA:** Separate durable workspace identity from disposable compute so restore and replacement do not redefine the user's data handle.
- **Pitfall or unknown:** The reviewed sources do not disclose a VM or separate-kernel boundary, and multi-attach storage creates concurrency and authorization semantics that SOMA must make explicit.
- **Primary sources:** [Serverless execution](https://archil.com/post/serverless-execution) and [The file system is the sandbox](https://archil.com/post/the-file-system-is-the-sandbox).

### Arker

- **Architecture and isolation:** **Unknown.** Arker says it built a new hypervisor and supports stateful resume, but the reviewed public page does not disclose its VMM, guest boundary, device model, jail, or network enforcement architecture.
- **Performance:** **Independent observation, 2026-08-28.** ComputeSDK recorded 310.93 ms median TTI, 349.67 ms p95, 358.76 ms p99, and 100% success.
- **Published claim:** **Vendor claim.** Arker says it resumes durable environments from cold storage faster than competitors resume warm environments, but the reviewed page does not provide a reproducible method for that comparison.
- **Best insight for SOMA:** Durable process and filesystem state should be a first-class product requirement rather than an accidental property of a long-lived host process.
- **Pitfall or unknown:** The strongest isolation and speed statements cannot be evaluated until the provider publishes mechanism, threat model, measurement boundary, and reproducible evidence.
- **Primary source:** [Arker](https://arker.ai/).

### Declaw

The official product spelling reviewed here is Declaw, while some supplied material used DeClaw.

- **Architecture and isolation:** **Verified design disclosure.** Declaw documents one Firecracker microVM per sandbox, a shared read-only base ext4 image, a per-sandbox writable overlay, preallocated network namespace and TAP slots, an in-guest `envd`, a private control path, a host-side egress proxy, and snapshot restore.
- **Template model:** **Verified design disclosure.** Declaw documents `base`, `python`, `node`, and `ai-agent` root filesystem templates plus asynchronous custom builds from Dockerfile-like input. Completed custom templates are immutable, and sandbox creation rejects a template that is not ready.
- **Agent interface:** **Verified design disclosure.** Its MCP wrapper runs an arbitrary stdio MCP server inside a sandbox, forwards only selected environment names and files, uses deny-all egress unless destinations are allowed, bridges stdio, and destroys the sandbox when the client disconnects.
- **Secrets:** **Verified design disclosure.** Declaw distinguishes values delivered into the guest from vault-backed credentials inserted by a host-side egress proxy for scoped destinations.
- **Performance:** **Independent observation, 2026-08-28.** ComputeSDK recorded 476.53 ms median TTI, 573.91 ms p95, 575.96 ms p99, and 100% success.
- **Published claim:** **Vendor claim.** Declaw publishes approximately 125 ms cold boot and approximately 30 ms snapshot restore, which use a different boundary from the ComputeSDK TTI result.
- **Best insight for SOMA:** Keep template builds outside Launch, preallocate namespace, veth, TAP, and policy resources outside the critical restore path, and support host-side credential injection when a protocol can be safely mediated.
- **Pitfall or unknown:** Snapshot artifacts require authenticated provenance and restore-time uniqueness repair, while a transparent proxy also needs explicit bypass resistance and trust-store policy.
- **Primary sources:** [Firecracker architecture](https://docs.declaw.ai/architecture/firecracker), [templates](https://docs.declaw.ai/features/templates), [networking](https://docs.declaw.ai/features/networking), [credential vault](https://docs.declaw.ai/security/credential-vault), and [MCP sandboxing](https://docs.declaw.ai/cli/mcp).
- **Detailed research:** [Declaw public architecture research](docs/research/declaw.md).

### Blaxel

- **Architecture and isolation:** **Verified design disclosure.** Blaxel describes individually isolated lightweight virtual machines, an in-sandbox API service, memory and filesystem preservation during scale-to-zero, and an in-memory root filesystem.
- **Performance:** **Independent observation, 2026-08-28.** ComputeSDK recorded 516.24 ms median TTI, 585.30 ms p95, 626.31 ms p99, and 100% success.
- **Published claim:** **Vendor claim.** Blaxel advertises standby resume under 25 ms, which is not the same measurement as create-to-first-command TTI.
- **Best insight for SOMA:** Represent standby as a distinct machine state with its own readiness and billing semantics.
- **Pitfall or unknown:** The reviewed sources do not identify the VMM, host jail, snapshot identity repair, or independent security review.
- **Primary sources:** [Sandbox overview](https://docs.blaxel.ai/Sandboxes/Overview) and [Blaxel Sandbox](https://blaxel.ai/sandbox).

### Vercel

- **Architecture and isolation:** **Verified design disclosure.** Vercel documents each sandbox as an isolated container inside a Firecracker microVM, with OCI image support and root privileges confined inside the VM boundary.
- **Performance:** **Independent observation, 2026-08-28.** ComputeSDK recorded 536.74 ms median TTI, 704.07 ms p95, 826.20 ms p99, and 100% success.
- **Published claim:** **Vendor claim.** Vercel documents filesystem snapshot optimizations but the reviewed primary pages do not publish a directly comparable current TTI result.
- **Best insight for SOMA:** Preserve OCI compatibility inside a smaller VM boundary instead of making container isolation itself the hostile-tenant boundary.
- **Pitfall or unknown:** A filesystem snapshot is not proof of process readiness, and the public product description is not a substitute for a threat model or isolation audit.
- **Primary sources:** [Vercel Sandbox](https://vercel.com/sandbox), [general availability announcement](https://vercel.com/changelog/vercel-sandboxes-ga), and [snapshot optimization](https://vercel.com/blog/optimizing-vercel-sandbox-snapshots).

### Superserve

- **Architecture and isolation:** **Verified design disclosure.** Superserve documents Firecracker microVMs, an Ubuntu 24.04 base, pause and resume checkpoints, custom templates, egress rules, and durable forkable machines.
- **Performance:** **Independent observation, 2026-08-28.** ComputeSDK recorded 486.54 ms median TTI, 814.03 ms p95, 868.24 ms p99, and 100% success.
- **Published claim:** **Vendor claim.** Superserve presents indefinite pause, resume, and fork as product capabilities but the reviewed pages do not publish a directly comparable TTI distribution.
- **Best insight for SOMA:** Long-lived agent state and image templates deserve stable handles that survive control-client disconnection.
- **Pitfall or unknown:** The reviewed sources do not disclose restore mechanics, uniqueness repair, host hardening, or whether the self-hosted and managed isolation contracts are identical.
- **Primary sources:** [Superserve introduction](https://docs.superserve.ai/introduction) and [Superserve](https://www.superserve.ai/).

### Modal

- **Architecture and isolation:** **Verified design disclosure.** Modal says default Sandboxes run under gVisor, while its beta VM Sandbox runtime provides a real Linux kernel for workloads requiring Docker, systemd, eBPF, FUSE, or cgroups.
- **Performance:** **Independent observation, 2026-08-28.** ComputeSDK recorded 849.24 ms median TTI, 949.48 ms p95, 959.87 ms p99, and 100% success.
- **Published claim:** **Vendor documentation.** Default networking denies inbound access and supports outbound controls, but the reviewed pages do not identify which runtime mode the benchmark adapter exercised.
- **Best insight for SOMA:** Expose backend capabilities through a stable contract while refusing to imply that a userspace kernel and a hardware VM have equivalent compatibility or isolation.
- **Pitfall or unknown:** Benchmark and security records must name the selected runtime because gVisor and the beta VM mode have different kernels, features, and trust boundaries.
- **Primary sources:** [Sandbox networking](https://modal.com/docs/guide/sandbox-networking) and [VM Sandboxes](https://modal.com/docs/guide/vm-sandboxes).

### Google Cloud Run

- **Architecture and isolation:** **Verified design disclosure.** Google documents first-generation Cloud Run as a gVisor-based sandbox and second-generation Cloud Run as a microVM environment with broader Linux compatibility.
- **Performance:** **Independent observation, 2026-08-28.** ComputeSDK recorded 682.17 ms median TTI, 1,170.27 ms p95, 1,285.20 ms p99, and 100% success.
- **Published claim:** **Vendor documentation.** Google states that second-generation execution can have longer cold starts for some services, but the benchmark record reviewed here does not identify the selected generation.
- **Best insight for SOMA:** Make the execution environment and its compatibility contract explicit in every machine inspection and benchmark record.
- **Pitfall or unknown:** Results cannot be compared safely when the adapter does not disclose whether it selected gVisor or the microVM environment.
- **Primary sources:** [Execution environments](https://cloud.google.com/run/docs/configuring/execution-environments) and [Cloud Run security](https://cloud.google.com/run/docs/securing/security).

### Runloop

- **Architecture and isolation:** **Verified design disclosure.** Runloop describes ephemeral virtual machines with snapshot, suspend, and resume, and its security page describes a dedicated microVM on bare metal with an inner container, lifecycle-specific egress controls, and credential-gateway tokens.
- **Performance:** **Independent observation, 2026-08-28.** ComputeSDK recorded 1,058.37 ms median TTI, 1,128.52 ms p95, 1,131.07 ms p99, and 100% success.
- **Published claim:** **Vendor documentation.** The reviewed pages describe hardware-backed isolation but do not publish a directly comparable TTI distribution.
- **Best insight for SOMA:** Keep long-lived credentials outside the guest and issue short-lived, policy-scoped tokens through a broker.
- **Pitfall or unknown:** The reviewed pages do not name the VMM or publish the snapshot and restore identity contract.
- **Primary sources:** [Devbox overview](https://docs.runloop.ai/docs/devboxes/overview) and [security and compliance](https://runloop.ai/security-compliance).

### E2B

- **Architecture and isolation:** **Verified design disclosure.** E2B's open infrastructure describes a Firecracker microVM per sandbox, prebooted snapshots in object storage, lazy memory restoration with `userfaultfd`, copy-on-write root filesystems, and an in-guest environment daemon.
- **Performance:** **Independent observation, 2026-08-28.** ComputeSDK recorded 1,085.47 ms median TTI, 1,306.85 ms p95, 1,374.73 ms p99, and 100% success.
- **Published claim:** **Vendor documentation.** E2B presents Firecracker as its isolation boundary, while its public architecture lets readers inspect the control-plane and node-local restore path.
- **Best insight for SOMA:** Lazy page restore can remove memory copy from the readiness path, while node-local ownership can remain behind a narrow control seam.
- **Pitfall or unknown:** `userfaultfd`, object storage, and distributed orchestration add attack and failure seams that demand artifact authentication, compatibility checks, and deterministic failure handling.
- **Primary sources:** [E2B infrastructure architecture](https://github.com/e2b-dev/infra/blob/main/docs/ARCHITECTURE.md) and [E2B](https://e2b.dev/).

### Daytona

- **Architecture and isolation:** **Verified design disclosure.** Daytona documents a control plane and runners, with default Linux sandboxes using OCI containers and namespaces, dedicated resource allocation, OCI-registry snapshots, and shared object-backed volumes, plus optional dedicated Linux and Windows VM classes.
- **Performance:** **Independent observation, 2026-08-28.** ComputeSDK recorded 326.21 ms median TTI, 481.78 ms p95, 492.68 ms p99, and 91% success.
- **Published claim:** **Vendor documentation.** Daytona distinguishes default container sandboxes from optional dedicated VMs, but the public benchmark record reviewed here does not name the selected class.
- **Best insight for SOMA:** Treat success rate as a first-order latency result and document control-plane, runner, storage, and execution boundaries separately.
- **Pitfall or unknown:** A default shared-kernel container does not meet SOMA's hostile multi-tenant VM boundary, and a fast percentile from only successful requests can obscure failed creates.
- **Primary sources:** [Daytona architecture](https://www.daytona.io/docs/en/architecture/) and [Daytona sandboxes](https://www.daytona.io/docs/en/sandboxes/).

### Tensorlake

- **Architecture and isolation:** **Verified design disclosure.** Tensorlake documents isolated microVMs backed by Firecracker and Cloud Hypervisor, with suspend and resume preserving memory and filesystem state.
- **Performance:** **Independent observation, 2026-08-28.** ComputeSDK recorded 1,404.60 ms median TTI, 2,431.01 ms p95, 2,431.96 ms p99, and 100% success.
- **Published claim:** **Vendor claim.** Tensorlake says its default image can start in hundreds of milliseconds and a systemd-based image in about one second, which is not the same as the observed external TTI boundary.
- **Best insight for SOMA:** A backend seam can select Firecracker or Cloud Hypervisor by workload capability without changing the machine lifecycle interface.
- **Pitfall or unknown:** Every performance and security record must identify which VMM, image, and restore mode supplied the result.
- **Primary source:** [Tensorlake Sandboxes introduction](https://docs.tensorlake.ai/sandboxes/introduction).

### Tenki by Luxor

Tenki is a product of Luxor Technology and is not a Tencent product.

- **Architecture and isolation:** **Verified design disclosure.** Tenki documents a full Linux VM with a dedicated kernel and hardware isolation, memory and filesystem preservation across pause and resume, and Linux x64 jobs on bare-metal AMD EPYC hosts.
- **Performance:** **Independent observation, 2026-08-28.** ComputeSDK recorded 1,674.37 ms median TTI, 2,162.74 ms p95, 2,171.38 ms p99, and 100% success.
- **Published claim:** **Vendor claim.** Tenki advertises pause and resume in under two seconds, which is distinct from the external create-to-first-command measurement.
- **Best insight for SOMA:** Record product ownership and backend identity precisely, and bind each session to isolated, revocable credentials rather than inherited host secrets.
- **Pitfall or unknown:** The reviewed pages do not disclose the VMM, jail model, snapshot artifact format, or independent security audit.
- **Primary sources:** [Tenki Sandbox](https://tenki.cloud/products/sandbox), [Tenki quickstart](https://tenki.cloud/docs/sandbox/quickstart), [Tenki security](https://tenki.cloud/docs/trust/security), and [Luxor Technology](https://luxor.tech/).

### Sail

- **Architecture and isolation:** **Verified design disclosure.** Sail describes a persistent, full-kernel-isolated Linux VM with root access, independent disks, local NVMe, Docker support, pause, resume, fork, and no fixed lifetime cap.
- **Performance:** **Independent observation, 2026-08-28.** ComputeSDK recorded 2,683.83 ms median TTI, 2,960.78 ms p95, 2,988.61 ms p99, and 100% success.
- **Published claim:** **Vendor claim.** Sail publishes long-horizon durability and cost comparisons, but those studies are vendor-authored and do not replace a third-party workload trace.
- **Best insight for SOMA:** Long-running machines need durable identity, typed errors, reconnectable control, and lifecycle semantics that do not assume short functions.
- **Pitfall or unknown:** The reviewed pages do not identify the VMM, hypervisor hardening, snapshot mechanics, or independent isolation evidence.
- **Primary sources:** [Sailboxes](https://sail.computer/sailboxes), [persistent sandbox announcement](https://www.sailresearch.com/blog/introducing-sailboxes-persistent-sandboxes), and [Sail SDK package metadata](https://registry.npmjs.org/%40sailresearch%2Fsdk/latest).

### Cloudflare

- **Architecture and isolation:** **Verified design disclosure.** Cloudflare describes each Sandbox as its own secure Cloudflare Container, addressed through a Worker and Durable Object lifecycle, with filesystem, process, and network controls supplied by the container platform.
- **Performance:** **Independent observation, 2026-08-28.** ComputeSDK recorded 5,255.33 ms median TTI, 6,714.63 ms p95, 7,408.34 ms p99, and 100% success.
- **Published claim:** **Vendor documentation.** The reviewed pages identify a container product but do not disclose a dedicated guest kernel or KVM boundary for each sandbox.
- **Best insight for SOMA:** A durable addressable control object can own lifecycle and routing while the execution machine remains replaceable.
- **Pitfall or unknown:** A container boundary must not be presented as equivalent to SOMA's KVM boundary without evidence of a separate kernel and hostile-tenant threat model.
- **Primary sources:** [Sandbox concepts](https://developers.cloudflare.com/sandbox/concepts/), [Sandbox SDK](https://github.com/cloudflare/sandbox-sdk), and [Cloudflare Sandboxes](https://www.cloudflare.com/products/sandboxes/).

### Upstash

- **Architecture and isolation:** **Verified design disclosure.** Upstash Box documents a dedicated Docker container with filesystem, process, and network isolation, auto-pause with durable state, and outbound network access enabled by default.
- **Performance:** **Independent observation, 2026-08-28.** ComputeSDK recorded 5,272.94 ms median TTI, 7,935.29 ms p95, 8,007.29 ms p99, and 100% success.
- **Published claim:** **Vendor documentation.** Upstash describes active-CPU billing and durable box state but does not disclose a separate guest kernel for each box.
- **Best insight for SOMA:** Make active, paused, and durable-state billing boundaries legible and machine-readable.
- **Pitfall or unknown:** Shared-kernel Docker isolation and default broad outbound access are incompatible with SOMA's hostile multi-tenant posture unless another undisclosed isolation layer and fail-closed policy exist.
- **Primary source:** [How Upstash Box works](https://upstash.com/docs/box/overall/how-it-works).

### Run Cloud

The benchmark and provider spell the name Run Cloud, and it must not be confused with the unrelated RunCloud server-management service at `runcloud.io`.

- **Architecture and isolation:** **Verified design disclosure.** Run Cloud states that its Linux sandboxes use Firecracker and KVM microVMs rather than shared-kernel containers and exposes adapters for sandbox APIs.
- **Performance:** **Independent observation, 2026-08-28.** ComputeSDK recorded 11,808.46 ms median TTI, 22,603.99 ms p95, 26,349.86 ms p99, 100% success, and a composite score of zero.
- **Published claim:** **Vendor documentation.** The product page states the isolation mechanism but the reviewed primary source does not publish a comparable distribution for the adapter path.
- **Best insight for SOMA:** Compatibility adapters should translate lifecycle and error contracts without leaking backend-specific internals into the core machine interface.
- **Pitfall or unknown:** The current independent tail is high, while the reviewed page does not disclose host hardening, snapshot design, or an external security assessment.
- **Primary source:** [Run Cloud](https://run.cloud/).

### Lightning AI

- **Architecture and isolation:** **Verified design disclosure with mechanism unknown.** Lightning AI documents a strongly isolated Linux environment with separate filesystem, network, and process views, OCI images, snapshots, and persistence, but the reviewed page does not name the VM, container, or userspace-kernel mechanism.
- **Performance:** **Independent observation, 2026-08-28 and 2026-08-21.** The current run had zero successful samples, while the 2026-08-21 run recorded 432.92 ms median TTI, 486.58 ms p95, 502.99 ms p99, and 100% success.
- **Published claim:** **Vendor documentation.** The product page states strong isolation but does not provide enough architecture detail to map that phrase to SOMA's KVM threat boundary.
- **Best insight for SOMA:** Preserve failed and stale benchmark states explicitly so automation never promotes an old healthy result into current evidence.
- **Pitfall or unknown:** The isolation mechanism, host boundary, VMM if any, and cause of the current failed run remain unknown from the reviewed primary sources.
- **Primary source:** [Lightning AI agent sandboxes](https://lightning.ai/docs/examples/agents/agent-sandboxes).

### RunPod

RunPod is included as a GPU and serverless compute reference rather than as a row in the reviewed 1 vCPU sandbox benchmark.

- **Use-case categories:** **Verified design disclosure.** Pods target interactive and stateful GPU or CPU environments reached through SSH, JupyterLab, or VS Code, Serverless endpoints target queued or load-balanced request workloads such as model inference, and Hub plus templates target reusable deployment packaging and discovery.
- **Products and control surface:** **Verified design disclosure.** RunPod Pods provide user-controlled GPU or CPU containers, Serverless endpoints autoscale requests across worker containers, and the Hub plus template system packages Docker images, hardware requirements, ports, environment, storage, and startup commands for reuse.
- **Serverless worker architecture:** **Verified design disclosure.** A worker is a containerized environment built from a Docker image supplied through a registry or GitHub build, attached to an endpoint, and moved through initializing, idle, running, throttled, outdated, and unhealthy states by the Serverless control plane.
- **Flex versus Active workers:** **Verified design disclosure.** Flex workers scale to zero when idle and incur a cold start when demand returns, while Active workers run continuously, avoid that cold-start path, bill while idle, and are intended for consistent or latency-sensitive traffic.
- **Hub and templates:** **Verified design disclosure.** Pod templates may be official, community, or custom, while Hub repositories use GitHub releases, `hub.json`, and `tests.json` and pass an automated build and test stage plus manual review before publication.
- **Compute shapes:** **Verified design disclosure.** The Pod API selects `GPU` or `CPU`, a GPU count and acceptable GPU models, or CPU flavor identifiers, while the current catalog spans NVIDIA and AMD devices from small workstation cards through data-center accelerators such as H200, B200, B300, and MI300X.
- **Storage:** **Verified design disclosure and vendor claim.** Network volumes persist independently of compute, mount at `/workspace` for Pods and `/runpod-volume` for Serverless workers, and are advertised at 200 MB/s to 400 MB/s with peaks up to 10 GB/s.
- **Storage constraint:** **Verified design disclosure.** A network volume constrains placement to its data center, separate volumes do not synchronize automatically, and simultaneous writers to one volume can corrupt data unless the application coordinates access.
- **Isolation:** **Verified design disclosure with mechanism limits.** RunPod says Pods and workers use containerized multi-tenant isolation, its compliance page says Secure Cloud and Serverless support Docker containers, and the reviewed sources do not disclose a per-workload VM, separate guest kernel, VMM, jail, or syscall-interposition boundary.
- **Secure Cloud distinction:** **Vendor claim.** RunPod recommends Secure Cloud for sensitive work and describes dedicated hardware or GPU placement there, while Community Cloud may share a host and relies on software container isolation.
- **Serverless lifecycle:** **Verified design disclosure.** A request can queue while a Flex worker cold-starts, endpoint limits bound concurrent workers and job lifetime, and FlashBoot retains paused worker state so later requests can revive it instead of pulling the image and loading the model again.
- **Startup performance:** **Vendor claim, 2026-08-25.** RunPod says FlashBoot can resume a prewarmed paused GPU worker in about 200 ms and under 200 ms in favorable cases, while a true GPU worker cold start can take roughly 30 seconds to more than two minutes depending on image and model loading.
- **Comparison boundary:** **Unknown for SOMA TTI.** No primary or independent result reviewed here measures a RunPod 1 vCPU sandbox from create request through first successful command, so GPU worker resume, endpoint deployment, model load, and inference latency must not be compared with the ComputeSDK sandbox table.
- **Best insight for SOMA:** Keep compute shape selection, artifact template, persistent volume, worker warming, and placement availability as separate capabilities so schedulers can optimize them without widening the one-machine VMM module.
- **Additional insight for SOMA:** Certify immutable image digests and test manifests rather than treating a mutable image tag, community template, or reviewed Hub listing as proof of artifact provenance.
- **Pitfall or unknown:** Containerized multi-tenancy does not satisfy SOMA's hostile KVM boundary, resource availability varies by model and data center, and persistent multi-writer storage plus warm GPU state add consistency and accounting semantics absent from a small CPU sandbox.
- **Primary sources:** [Pod overview](https://docs.runpod.io/pods/overview), [Pod creation API](https://docs.runpod.io/api-reference/pods/POST/pods), [Serverless overview](https://docs.runpod.io/serverless/overview), [worker architecture and modes](https://docs.runpod.io/serverless/workers/overview), [Serverless pricing and worker types](https://docs.runpod.io/serverless/pricing), [endpoint settings](https://docs.runpod.io/serverless/endpoints/endpoint-configurations), [GPU types](https://docs.runpod.io/references/gpu-types), [CPU types](https://docs.runpod.io/references/cpu-types), [network volumes](https://docs.runpod.io/storage/network-volumes), [Pod templates](https://docs.runpod.io/pods/templates/overview), [Hub overview](https://docs.runpod.io/hub/overview), [security and compliance](https://docs.runpod.io/references/security-and-compliance), [Secure Cloud security guidance](https://www.runpod.io/articles/guides/keep-data-secure-cloud-gpus), and [FlashBoot architecture claim](https://www.runpod.io/blog/serverless-gpu-cold-starts-flashboot).

## Tencent, CubeSandbox, and Dragonball attribution

Tencent's relevant project in this ledger is CubeSandbox.

Tenki belongs to Luxor Technology, not Tencent.

Dragonball is a Kata Containers VMM created by Ant Group, while the current CubeSandbox architecture names CubeHypervisor built on RustVMM and KVM.

No reviewed primary source establishes Dragonball as CubeSandbox's VMM, so this ledger does not merge those architectures.

### Tencent CubeSandbox

- **Architecture and isolation:** **Verified design disclosure.** CubeSandbox documents one KVM microVM and guest kernel per sandbox, CubeHypervisor on RustVMM, a seccomp-minimized VMM surface, a node-local data plane, a Redis-coordinated control plane, XFS `FICLONE` copy-on-write storage, eBPF networking, and a host-side L7 egress and credential proxy.
- **Performance:** **Project claim, reviewed 2026-08-28.** The architecture page claims sub-100 ms cold starts from presnapshotted templates and a RustVMM restore path, while the repository overview claims a fully serviceable sandbox in under 60 ms and under 5 MB of memory overhead.
- **Best insight for SOMA:** CubeHypervisor, node-local ownership, reflink storage, eBPF policy, and host-side secret injection are separable modules that can sit behind narrow seams.
- **Pitfall or unknown:** CubeSandbox's Redis, scheduler, proxy, containerd, and cluster control plane solve a broader problem than SOMA's one-machine process, so copying the topology would add unnecessary depth and failure modes to the local core.
- **Primary sources:** [CubeSandbox architecture](https://github.com/TencentCloud/CubeSandbox/blob/master/docs/architecture/overview.md) and [CubeSandbox repository](https://github.com/TencentCloud/CubeSandbox).

### Dragonball in Kata Containers

- **Architecture and isolation:** **Verified design disclosure.** Kata Containers documents Dragonball as a Rust KVM VMM integrated into the Rust containerd shim process, with x86_64 and aarch64 support, a unified lifecycle, and no VMM IPC boundary.
- **Performance:** **Project claim, reviewed 2026-08-28.** Kata documents lower startup and IPC overhead from the built-in design but the reviewed primary pages do not publish a directly comparable TTI distribution.
- **Best insight for SOMA:** An in-process VMM library can remove a control IPC seam when the owning process and VMM intentionally share fate.
- **Pitfall or unknown:** The Kata and CRI integration surface is much broader than SOMA's initial one-process, one-machine contract, and removing IPC also combines faults and privileges in one process.
- **Primary sources:** [Kata virtualization design](https://github.com/kata-containers/kata-containers/blob/main/docs/design/virtualization.md), [Kata hypervisors](https://github.com/kata-containers/kata-containers/blob/main/docs/hypervisors.md), and [Dragonball source](https://github.com/kata-containers/kata-containers/tree/main/src/dragonball).

## Open-source architecture references

These projects are architecture references rather than a claim that each is a hosted competitor or production-ready hostile multi-tenant runtime.

### Firecracker

- **Architecture and isolation:** **Verified design disclosure.** Firecracker uses KVM, a deliberately small device model, one VMM process per microVM, a jailer, seccomp filters, cgroups, and namespaces.
- **Performance:** **Project specification, reviewed 2026-08-28.** The performance specification defines an API response requirement of 8 CPU ms, typical wall time of 12 ms, guest-init boot at or below 125 ms for a defined configuration, and VMM overhead at or below 5 MiB.
- **Best insight for SOMA:** Keep the device surface small, make one process own one VM, and place jail, resource, and syscall policy outside guest-controlled state.
- **Pitfall or unknown:** Firecracker provides mechanisms rather than SOMA's complete security boundary, artifact chain, network policy, restore identity repair, or readiness protocol.
- **Primary sources:** [Firecracker design](https://github.com/firecracker-microvm/firecracker/blob/main/docs/design.md) and [performance specification](https://github.com/firecracker-microvm/firecracker/blob/main/SPECIFICATION.md).

### Cloud Hypervisor

- **Architecture and isolation:** **Verified design disclosure.** Cloud Hypervisor is a Rust VMM built from RustVMM components for modern 64-bit cloud workloads, with KVM and Microsoft Hypervisor backends, x86_64 and aarch64 support, and a broader device and lifecycle set than Firecracker.
- **Performance:** **Unknown for this comparison, reviewed 2026-08-28.** The primary project page reviewed here does not publish a current create-to-first-command distribution comparable to ComputeSDK.
- **Best insight for SOMA:** Preserve an architecture and hypervisor-backend seam early so aarch64 and richer virtio needs do not force a lifecycle API rewrite.
- **Pitfall or unknown:** Hotplug, migration, and broader device support create additional code, compatibility, and attack surface that SOMA should not enable before a requirement demands them.
- **Primary source:** [Cloud Hypervisor](https://github.com/cloud-hypervisor/cloud-hypervisor).

### Unikraft

- **Architecture and isolation:** **Verified design disclosure.** Unikraft composes application-specific unikernels from small libraries and exposes configurable system APIs without requiring a conventional general-purpose guest distribution.
- **Performance:** **Project and academic claim, reviewed 2026-08-28.** The Unikraft paper reports approximately 1 MB images, less than 10 MB memory for evaluated applications, approximately 1 ms application boot above the VMM, and approximately 3 ms to 40 ms total boot depending on configuration.
- **Best insight for SOMA:** Specializing the guest boot path can remove work more effectively than tuning a general-purpose distribution after the fact.
- **Pitfall or unknown:** A unikernel is not a transparent replacement for arbitrary OCI Linux workloads, package managers, systemd, or broad syscall compatibility.
- **Primary sources:** [Unikraft architecture](https://unikraft.org/docs/internals/architecture) and [Unikraft paper](https://arxiv.org/abs/2104.12721).

### Zeroboot

- **Architecture and isolation:** **Verified design disclosure.** Zeroboot maps a Firecracker memory snapshot with `mmap(MAP_PRIVATE)`, creates a separate KVM VM, restores CPU state, and relies on copy-on-write pages for child-local writes.
- **Performance:** **Project benchmark, reviewed 2026-08-28.** The project reports 0.79 ms p50 and 1.74 ms p99 spawn latency, approximately 8 ms for fork plus Python execution, and 815 ms for 1,000 concurrent forks.
- **Best insight for SOMA:** `MAP_PRIVATE` restore can make memory cloning a virtual-memory operation and is directly relevant to SOMA's planned restore path.
- **Pitfall or unknown:** The repository labels itself a working prototype and lists duplicated CSPRNG or userspace PRNG state, one vCPU, no networking, and roughly 15 second template rebuilds as limitations.
- **Primary source:** [Zeroboot](https://github.com/zerobootdev/zeroboot).

### Mitos

- **Architecture and isolation:** **Verified design disclosure.** Mitos runs Firecracker VMs either in unprivileged Kubernetes husk pods or through an in-process `forkd` engine and implements live copy-on-write VM forks.
- **Performance:** **Project benchmark, reviewed 2026-08-28.** Mitos reports approximately 27 ms p50 warm activation, 96.8 ms hosted TTI from its own harness, approximately 104 ms fork-to-first-exec on its reference node, and approximately 67 ms fork-to-first-exec with reflink and NVMe.
- **Best insight for SOMA:** Name the measurement boundary and engine path for every number, and never place a local engine restore beside a remote provider TTI without labeling the category difference.
- **Pitfall or unknown:** The project explicitly notes that Kubernetes pod policy governs the husk pod rather than automatically governing the workload inside the VM, and its hosted TTI is not a ComputeSDK leaderboard rank.
- **Primary source:** [Mitos repository and benchmark record](https://github.com/mitos-run/mitos).

### SporeVM

- **Architecture and isolation:** **Verified design disclosure.** SporeVM describes an aarch64 VM checkpoint and fork primitive with a shared board, CLI, and API across Linux KVM and Apple Silicon HVF, plus content-addressed checkpoint materials and compatible-host restore classes.
- **Performance:** **Project benchmark status, reviewed 2026-08-28.** The project says fast fork is its benchmark and publishes live demonstrations, but this ledger did not record a stable, directly comparable percentile distribution from the reviewed primary page.
- **Best insight for SOMA:** Bind snapshot compatibility to a declared host class and verify every memory, rootfs, disk, and device-state component before restore.
- **Pitfall or unknown:** The project explicitly does not claim hardened public-cloud multi-tenant isolation, so it is architecture evidence rather than a security baseline for SOMA.
- **Primary sources:** [SporeVM](https://sporevm.com/) and [SporeVM repository](https://github.com/sporevm/sporevm).

### Machinen

- **Architecture and isolation:** **Verified design disclosure.** Machinen Runtime documents native aarch64 HVF and Linux KVM or amd64 Linux KVM backends, whole-VM snapshot and fork, copy-on-write disks, vsock execution, same-architecture restore, entropy reseeding, and non-inheritance of conflicting host port forwards.
- **Performance:** **Unknown for this comparison, reviewed 2026-08-28.** The project has an open benchmark-dashboard issue rather than a current primary-source TTI distribution suitable for this ledger.
- **Best insight for SOMA:** Specify restore behavior for entropy, timers, sockets, port forwards, machine identity, and architecture compatibility rather than treating memory restoration as sufficient.
- **Pitfall or unknown:** Public benchmark evidence and a hardened hostile-tenant security claim remain incomplete, so feature completeness must not be mistaken for provider-grade isolation evidence.
- **Primary sources:** [Machinen context](https://github.com/redwoodjs/machinen/blob/main/CONTEXT.md), [snapshot, restore, and fork guide](https://github.com/redwoodjs/machinen/blob/main/docs/guides/snapshot-restore-fork.md), and [benchmark dashboard issue](https://github.com/redwoodjs/machinen/issues/951).

### libkrun and Microsandbox

- **Architecture and isolation:** **Verified design disclosure.** libkrun is an embeddable virtualization library using KVM on Linux and HVF on Apple Silicon, while Microsandbox layers cross-platform OCI-oriented microVM workflows and an embeddable API over libkrun without requiring a long-running daemon.
- **Performance:** **Project claim, reviewed 2026-08-28.** Microsandbox reports average guest boot below 100 ms on an Apple M1, which is neither an independent result nor an end-to-end create-to-first-command measurement.
- **Best insight for SOMA:** An embeddable library and no-daemon control surface are strong locality properties, while the KVM and HVF adapters can remain behind the same lifecycle seam.
- **Pitfall or unknown:** libkrun states that the guest and VMM belong to the same security context, so its documented model alone is insufficient evidence for hostile provider multitenancy.
- **Primary sources:** [libkrun](https://github.com/libkrun/libkrun) and [Microsandbox](https://github.com/superradcompany/microsandbox).

## Lessons SOMA should carry forward

1. Measure from an authenticated create request through the first successful guest command, and always publish median, p95, p99, success rate, concurrency, image, region, and backend.
2. Record cold boot, snapshot restore, standby wake, and full external TTI as separate metrics because collapsing them creates misleading comparisons.
3. Treat a failed or missing run as availability evidence rather than a latency of zero or a reason to silently reuse an older result.
4. Repair cloned entropy, instance identifiers, clocks, leases, network identity, host bindings, and inherited sockets before declaring a restored machine ready.
5. Authenticate kernel, rootfs, memory, device-state, policy, and compatibility metadata before mapping or executing a snapshot artifact.
6. Keep customer secrets out of guest memory and disk by using narrow host-side credential and egress brokers with deny-by-default policy.
7. Move namespace, TAP, address, route, and policy allocation off the critical path where bounded preallocation can preserve fail-closed semantics.
8. Preserve one-process, one-machine ownership and local failure depth even when an optional scheduler or hosted control plane is added above it.
9. Make every adapter report its isolation mode, VMM, guest architecture, snapshot mode, and unsupported capabilities instead of presenting all backends as equivalent.
10. Use `MAP_PRIVATE`, reflinks, and lazy page loading as implementation techniques behind a verified artifact and readiness contract rather than exposing them as product semantics.
11. Keep the device model and privileged host surface minimal, and add hotplug, migration, GPU, or nested-container support only behind explicit capability gates.
12. Treat source availability as inspectability rather than proof of security, and require threat modeling, fuzzing, patch discipline, and external review for hostile multitenancy.
13. Separate durable workspace identity from ephemeral compute identity so replacement, restore, and fork do not create ambiguous ownership.
14. Publish benchmark source, raw samples, commit revision, hardware, and measurement boundary so later research can distinguish a changed product from a changed harness.

## Maintenance protocol

- Update the research cut whenever any rank, architecture claim, or primary source changes.
- Prefer immutable source revisions for recorded measurements and use current documentation links for product behavior that may evolve.
- Preserve old measurements with their dates rather than overwriting history or comparing stale and current rows without labels.
- Add a provider only when its identity and primary source can be distinguished from similarly named products.
- Mark a mechanism unknown when public evidence is missing instead of deriving architecture from latency, marketing language, package names, or screenshots.
