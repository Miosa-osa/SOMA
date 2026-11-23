# ADR 0012: Fail-closed sandbox networking

- Status: Accepted
- Date: 2026-08-28

## Context

SOMA runs untrusted workloads inside hardware-isolated Machines, but hardware isolation does not define which packets a Machine may send or receive.
The portable caller must be able to request no network, public internet access, explicitly broader egress, DNS behavior, assigned addresses, and published ports without learning one host's firewall or virtual-device mechanisms.
Operators must also be able to define address pools, resolvers, proxies, protected destinations, and custom network adapters for their own cloud or on-premises environment.

Network intent is not proof of network enforcement.
A backend can accept a deny request while attaching a default network, preserve an image's resolver configuration while declining to configure DNS, or record a published port before any host listener exists.
SOMA therefore needs exact runtime evidence and exact cleanup evidence rather than a successful command exit alone.

The Apple Container adapter is useful for real VM-backed development on macOS, but its lifecycle and port-binding behavior cannot certify the Linux production design.
The initial production substrate remains a real Ubuntu 24.04 x86_64 KVM host.

## Decision

SOMA separates portable network intent, operator-owned network profiles, runtime activation, effective evidence, and cleanup evidence.
Each layer is typed, bounded, versioned where it crosses a process or durable-state seam, and fail-closed when required evidence is unavailable.

### Portable network intent

`NetworkPolicy` is the portable declaration of what one Machine may use.
It does not contain netlink messages, nftables expressions, shell fragments, cloud credentials, proxy credentials, or backend-specific device names.

The secure default is:

- Egress is `Denied`.
- DNS is `Denied`.
- No host ports are published.
- No externally reachable guest address is promised.
- No network or proxy profile is selected.

A backend must not silently replace this default with its runtime default.
An unspecified policy is permitted only for explicitly labeled development and discovery flows.
A production profile must reject unspecified security intent.

The portable egress meanings are:

- `Denied` permits no guest-originated IP egress and requires DNS to be denied.
- `PublicInternet` permits destinations classified as public by the certified operator profile while denying private, loopback, link-local, multicast, host, peer, control-plane, and metadata destinations.
- `Unrestricted` removes the portable public-destination restriction only when explicitly requested and admitted by the operator.

`Unrestricted` never bypasses non-optional host, peer, control-plane, or metadata protections.
If an operator wants narrower private access or proxy-only access, it defines a named profile rather than accepting caller-supplied firewall syntax.
Destination policy is enforced on the resolved destination address so DNS rebinding cannot turn a public-name decision into access to a protected address.

DNS is an independent policy dimension.
An attached interface does not imply DNS permission, and DNS permission does not imply general internet access.

The DNS meanings are:

- `Denied` configures no SOMA resolver and denies conventional DNS transport through the network policy.
- `System` uses the exact resolver set supplied by the selected certified operator profile rather than copying an unverified host loopback resolver.
- `Custom` uses a bounded exact list of caller-visible resolver IP addresses after operator admission.

DNS denial does not claim to identify names tunneled through otherwise allowed application protocols such as HTTPS.
The egress policy remains the authority for which resolved addresses a workload can reach.

Ingress is denied unless the request contains an explicit port publication.
Each publication identifies all of the following values:

- An exact IPv4 or IPv6 host bind address.
- An explicit IPv6 `v6_only` value for every IPv6 bind.
- A fixed nonzero host port or an automatic host-port request.
- A fixed nonzero guest port.
- TCP or UDP transport.

SOMA never relies on the host's `IPV6_V6ONLY` default.
IPv4 wildcard, IPv6 wildcard, dual-stack, and `v6_only` collision behavior is modeled before activation and confirmed by real exclusive socket binds.
The operator must own or admit every requested host bind address.

### Operator-defined network and proxy profiles

A caller may select an opaque, validated network profile ID and may explicitly disable profile-controlled networking.
A caller may also select a named proxy profile or explicitly disable proxy use.
Literal profile implementation details do not enter the portable Machine request.

An operator-defined network profile may provide:

- IPv4 and IPv6 address pools.
- Guest gateway and routing configuration.
- System resolver addresses and DNS search policy.
- Public and private destination classification.
- Mandatory protected host, peer, control-plane, and metadata ranges.
- Permitted host ingress addresses and port ranges.
- A named transparent or explicit proxy route.
- Rate, connection, and bandwidth limits.
- A custom adapter behind the network-runtime seam.

Dynamic address allocation is the default.
A requested static address is admitted only when the operator proves ownership, uniqueness, and reservation in the selected profile.
The exact assigned guest addresses are returned as effective evidence.

Proxy endpoints and authentication material are resolved from operator configuration or an operator secret store.
Secrets never appear in requests, durable Machine records, receipts, logs, or profile IDs.
Injecting an application proxy environment variable is not network enforcement because a workload can ignore it.
A proxy-enforced profile must make the proxy route the only permitted egress path at the network layer.

Profiles are versioned or content-addressed so evidence identifies the exact admitted policy generation.
A profile may tighten a request but must not silently grant access beyond the request.

### Network-runtime seam

Host-specific behavior sits behind one deep network-runtime interface.
The interface is intentionally smaller than any one implementation:

```text
acquire(owner, intent) -> ReservedNetworkLease
activate(lease, readiness) -> ActiveNetworkEvidence
inspect(lease) -> ActiveNetworkEvidence
release(lease, owner) -> NetworkCleanupEvidence
reconcile(expected_owners) -> ReconcileReport
```

`acquire` validates policy, reserves every address and port transactionally, and creates no reachable ingress.
`activate` requires authenticated guest readiness before it makes any requested ingress reachable.
`inspect` reads live substrate state rather than replaying stored launch intent.
`release` is idempotent, verifies exact ownership, and returns evidence for every resource it removes or proves absent.
`reconcile` compares the durable ownership ledger with kernel and runtime state after startup, crash, or operator repair.

Adapters may implement this seam for the Linux production broker, Apple Container development, a cloud VPC, or an on-premises network.
Every adapter must pass the same portable conformance contract before it can claim support for a policy dimension.
A future CNI adapter may translate this seam into CNI operations, but CNI is not SOMA's portable public interface.

### Exact effective evidence

Every Ready receipt carries `EffectiveNetwork` separately from requested `NetworkPolicy`.
Effective evidence records an observation as observed or unavailable rather than inventing a value.
An unavailable observation never satisfies an explicit restriction or publication request.

Effective network evidence includes:

- Whether an IP network is detached or attached.
- The effective egress class.
- The effective DNS policy and exact configured resolver addresses when applicable.
- The exact assigned IPv4 and IPv6 guest addresses.
- The selected network and proxy profile identities and admitted versions.
- Every exact host bind address, host port, guest port, and transport publication.
- The explicit `v6_only` value for every IPv6 publication.
- The ingress activation class.
- Proof that requested ingress became reachable only after authenticated readiness.

The activation classes distinguish at least `NotApplicable`, `AtomicSocketHandoff`, and `VerifiedRuntimeRebind`.
`AtomicSocketHandoff` means one owned bound socket remained reserved through activation without a close-and-rebind window.
`VerifiedRuntimeRebind` identifies a development adapter that had to release a reservation before another runtime bound the same endpoint and then verified the live result.

Stored launch evidence cannot substitute for a later live inspection.
Inspect must report drift, unavailable evidence, or ownership loss rather than repeating a prior receipt.

### Cleanup evidence

Network cleanup is part of Machine cleanup rather than a best-effort background detail.
The cleanup result reports the terminal disposition of all resources owned by the network lease.

The Linux cleanup evidence must cover at least:

- Guest address and IPAM lease release.
- Host port reservation and forwarding removal.
- Proxy route removal.
- DNS policy removal.
- nftables set and map membership removal.
- Conntrack-zone cleanup or bounded retirement.
- TAP, veth, bridge or routing-domain, and network-namespace removal.
- Durable lease transition to a terminal state.
- A final live inspection that finds no resource still owned by the Instance.

A port is not proven released merely because the Machine process exited.
The adapter must prove that no forwarding rule remains and that the exact endpoint can be exclusively reserved again, subject to the operating system's protocol semantics.
Uncertain cleanup keeps the Machine in a reaping or recovery-required state and blocks unsafe lease reuse.

### Apple Container development adapter

Apple Container remains a development adapter with a smaller evidence boundary.
Local Apple Container 1.3 probes established the following behavior on the project development host:

- `--network none` produced no configured network, no runtime network status, and no usable route.
- The default network attached NAT egress.
- Explicit `--dns` addresses were represented in inspection and could provide working resolution.
- Runtime-default DNS was not reliable on the tested host.
- `--no-dns` meant that the runtime did not configure DNS, but an image could retain resolver configuration and resolve names.
- Port `0` was rejected as an automatic host-port request.
- Port `1` was also rejected by the current parser.
- A fixed published port was staged at create time and bound only when the Machine started.
- Two creates could stage the same fixed port, with the conflict appearing when the second Machine started.
- Combining `--network none` with a publication did not produce a reachable host listener.

SOMA therefore never translates DNS denial to Apple Container `--no-dns`.
It rejects publications with a detached network and rejects any Apple policy dimension it cannot prove exactly.

For automatic development ports, SOMA reserves real sockets, creates the Apple Container Machine with the selected fixed values while holding those reservations, releases immediately before start, starts the exact owned Machine, inspects the exact configuration, and probes occupancy before reporting Ready.
This sequence has an unavoidable rebind race and must be labeled `VerifiedRuntimeRebind`.
Fixed-port conflicts never remap silently.
Automatic-port retry is bounded and applies only to a verified address-in-use race.

Apple results cannot satisfy the Linux production conformance, isolation, cleanup, or performance gates.

### Linux production topology

The production VMM remains unprivileged with respect to host networking.
It never receives `CAP_NET_ADMIN`, parses nftables input, mutates host routes, or allocates its own address.

The operator-facing host process communicates with a narrow privileged broker named `soma-netd` over a filesystem-protected Unix `SOCK_SEQPACKET` socket.
The protocol is bounded, typed, versioned, peer-authenticated, and idempotent by owner and operation identity.
It accepts structured intent only and contains no raw shell, netlink, nftables, or proxy configuration strings.

`soma-netd` owns:

- A durable lease ledger tied to host boot identity, Instance identity, operation identity, and request fingerprint.
- Per-Machine network namespaces.
- Per-Machine TAP and veth devices with a bounded forwarding domain between the guest and host.
- Collision-safe IPv4 and IPv6 IPAM.
- Unique MAC and network identity allocation.
- Per-Machine conntrack zones.
- The qualified host nftables ruleset and its dynamic sets and maps.
- DNS enforcement and resolver configuration.
- Transactional TCP and UDP host-port reservations.
- Ingress activation and deactivation.
- Startup and periodic reconciliation.

Host qualification installs and verifies a small constant default-deny nftables topology.
Machine lifecycle mutates bounded set and map elements transactionally instead of constructing per-request shell programs.
Anti-spoofing binds source addresses and interfaces to the owning lease.
Peer-to-peer forwarding is denied unless a named profile explicitly permits an admitted route.

Each Machine receives its own assigned addresses, namespace, TAP, veth path, and conntrack zone.
The broker opens the TAP and passes the already-open file descriptor to the unprivileged VMM with `SCM_RIGHTS`.
The VMM cannot use that descriptor to create or reconfigure other host network devices.

The protected-destination set includes the host, gateways, other tenants, control-plane endpoints, and every metadata endpoint defined by the certified substrate.
It includes the applicable AWS, Google Cloud, and Azure metadata addresses on those substrates.
These denials remain active for `PublicInternet` and `Unrestricted` egress.

Port reservations are transactional across the complete request.
Fixed ports receive one exclusive attempt.
Automatic ports use a bounded allocator and preserve the selected endpoint in durable lease state.
The production activation path must retain ownership through an atomic socket handoff or another conformance-proven race-free mechanism before it can claim `AtomicSocketHandoff`.

Ingress forwarding remains absent while the guest boots and repairs identity.
Only an authenticated successful guest readiness result allows `activate` to publish the reserved endpoints.
If activation or verification fails, the full network lease rolls back and Ready is never returned.

The broker reconciles durable leases against namespaces, devices, nftables state, addresses, sockets, forwarding state, and live VMM ownership after every restart.
Unknown resources are quarantined or removed only after exact SOMA ownership is proven.
Missing required resources invalidate the lease and trigger fail-closed cleanup rather than reconstruction from assumptions.

### Production conformance gate

Production network support is admitted only by retained end-to-end results from the exact Ubuntu 24.04 x86_64 host profile.
macOS tests, mocks, command-argument tests, and Linux namespace-only tests are supporting evidence rather than substitutes.

The gate covers at least:

- Secure default isolation with direct IPv4, IPv6, TCP, UDP, and DNS attempts.
- Exact `Denied`, `PublicInternet`, and `Unrestricted` behavior.
- System and custom DNS behavior, malformed replies, resolver failure, and DNS rebinding attempts.
- Protected AWS, Google Cloud, Azure, host, peer, control-plane, loopback, link-local, private, multicast, and broadcast destinations as applicable.
- Exact IPv4, IPv6, wildcard, and `v6_only` host-bind behavior.
- Fixed and automatic TCP and UDP publication.
- Multiple publications as one transaction.
- No ingress before authenticated readiness and immediate closure during termination.
- Static and dynamic address collision handling.
- Anti-spoofing and tenant-to-tenant isolation.
- Concurrent allocation, exhaustion, and deterministic recovery.
- Host-process, VMM, broker, forwarding-worker, and machine crash windows at every lease transition.
- Broker restart and host reboot reconciliation.
- Idempotent acquire, activate, release, and reconcile replay.
- Proof that failed and successful Machines leave no namespace, device, address, rule, socket, proxy, DNS, or ledger leak.
- Repeated performance measurements with the complete network preparation and activation boundary disclosed.

Any unavailable restriction evidence, ownership mismatch, incomplete cleanup, or uncontrolled port race fails the release gate.

## Alternatives considered

### Let each VMM configure host networking

This option was rejected because it would give every latency-sensitive VMM broad host authority and duplicate network policy, cleanup, and reconciliation across processes.
The narrow broker keeps privilege and kernel mutation behind one deep interface.

### Expose raw firewall or proxy configuration to callers

This option was rejected because it would make the portable interface backend-specific, create an injection surface, and move operator security invariants into untrusted requests.
Named versioned profiles provide customization without exposing credentials or arbitrary host programs.

### Treat runtime configuration as evidence

This option was rejected because declared DNS, network, and port values do not prove live routes, listeners, filtering, or cleanup.
SOMA requires exact live observations and explicit unavailable states.

### Use Apple Container behavior as the production model

This option was rejected because Apple Container uses a different host virtualization and networking stack and has a verified close-and-rebind window for published ports.
It remains a useful development adapter with an honestly smaller activation class.

### Publish ingress before guest readiness

This option was rejected because it exposes a guest during boot and identity repair, before SOMA can authenticate the intended Instance.
Reservation occurs early, but reachability begins only after readiness.

### Make CNI the portable public interface

This option was rejected because CNI does not express SOMA's full request, evidence, readiness-gated ingress, durable ownership, or receipt semantics.
CNI remains a possible adapter behind the network-runtime seam.

## Consequences

The portable caller receives secure defaults, exact addresses, exact publications, and truthful evidence without learning the host implementation.
Operators can create their own networks and proxy profiles while retaining authority over address ownership, credentials, protected destinations, and conformance.

The Linux network broker becomes security-critical and must be reviewed, fuzzed, load-tested, and recovered independently from the VMM.
The durable lease ledger and kernel reconciliation add implementation work, but they prevent silent port reuse, orphaned access, and fabricated cleanup.

Apple development supports only the subset it can prove.
A rejected policy is preferable to a successful launch with weaker networking than the caller requested.

The first production release remains blocked until the real Ubuntu host passes the complete networking conformance gate and retains its raw evidence.
