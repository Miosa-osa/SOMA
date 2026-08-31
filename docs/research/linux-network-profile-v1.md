# SOMA Linux network profile v1

## Decision

The privileged `soma-netd` broker owns network namespaces, TAP and veth devices, IPAM, routes, nftables, conntrack zones, DNS policy, proxy attachment, ingress reservations, and cleanup.
The unprivileged VMM receives one TAP descriptor through Unix `SOCK_SEQPACKET` plus `SCM_RIGHTS`.

## Prepared bundle and assignment

A sterile bundle may contain an unattached namespace, TAP, veth pair, conntrack zone, address lease reservation, nftables object handles, DNS endpoint reservation, and ingress reservation.
It contains no tenant destination, credential, public listener, or active forwarding.
Assignment atomically binds BundleId, InstanceId, OperationId, network-profile digest, MAC, addresses, protected routes, and cleanup generation in the durable ownership ledger.

Default policy denies attachment, DNS, egress, proxy, and ingress unless the admitted request and operator profile explicitly enable them.
Public Internet mode still blocks loopback, link-local, RFC1918 and ULA space, peers, control plane, host services, and cloud metadata endpoints.
DNS uses only declared resolvers and follows the same destination policy.

## Activation

The guest link remains down and ingress maps remain inactive during restore and repair.
After authenticated network repair, the broker verifies namespace, link, lease, route, nftables, resolver, and listener state and atomically activates forwarding.
Automatic ports are reserved before Launch and published only after Ready.
A reservation binds one exclusive socket that is never made to listen, so a reserved but unpublished port refuses a connection rather than accepting one nothing can serve.
Activation, the same step that enables forwarding, installs one publication table per bundle carrying the destination translation at the prerouting and output hooks and a masquerade out through the bundle veth; release deletes that table first and proves it gone by name.
The layer above a published port is decided in [Published ports and preview URLs](published-ports-and-preview-urls.md) and is not part of this broker.
Egress and ingress counters are bound to Instance identity, not a reusable interface name.

Proxy profiles transfer no raw credential to the VMM.
Operator-owned sidecars or gateways receive short-lived Instance-scoped authority outside guest memory.

## Control socket authority

The control socket is the only way to reach the privileged broker, so its authority is part of the profile rather than a deployment detail.

The broker creates the socket's parent directory itself, sets it to mode `0750` owned by the broker's own user identity and the operator-named socket group, and reads it back before it binds.
It binds inside that directory, sets the socket node to mode `0660` with the same owner and group, reads that back, and only then listens.
A path that already exists is removed only when it is a socket the broker already owns; a regular file, a directory, or another user's socket is a refusal rather than an unlink, so a stale path fails closed and a normal restart still succeeds.
Both nodes are verified again before every accept, so ownership drift or a later permission change fails closed instead of widening reach.

Every connection is authenticated from the kernel-derived peer credential rather than from any request field.
A peer the authority does not admit is closed before a single frame is read.
An admitted peer still needs the capability its exact operation requires.

| Capability | Operations | Production holder |
| --- | --- | --- |
| Lifecycle | Claim, Activate, Release | `soma-hostd` |
| Reconcile | Reconcile | operator repair tooling |

The handoff is therefore exact.
`soma-hostd` runs as the single lifecycle user identity, is a member of the socket group, claims one bundle per Machine, receives that bundle's TAP descriptor and its single-use activation challenge on the same connection, and is the only identity that may later activate or release that assignment: the broker records the claiming identity and refuses any other peer for the same bundle and generation.
The jailed VMM never speaks this protocol and holds no capability at all.
It receives one already-open TAP descriptor from `soma-hostd` and cannot create, reconfigure, or enumerate any host network device with it.
Operator tooling that only reconciles is a separate identity with no lifecycle authority.

## Cleanup and modules

Release disables ingress first, then removes forwarding, conntrack state, routes, addresses, veth, TAP, namespace, reservations, and ledger ownership.
Every operation is idempotent and reconcile compares durable intent with kernel reality after crashes.

Modules are `intent`, `profile`, `ipam`, `namespace`, `tap`, `firewall`, `dns`, `proxy`, `ingress`, `activate`, `release`, and `reconcile`.
Tests cover IPv4 and IPv6 deny-by-default, spoofing, metadata blocking, DNS bypass, peer isolation, ingress-before-Ready, port races, proxy failure, broker and VMM crashes, ambiguous retries, 100-way assignment, and complete cleanup latency.
