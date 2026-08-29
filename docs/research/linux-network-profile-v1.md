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
Egress and ingress counters are bound to Instance identity, not a reusable interface name.

Proxy profiles transfer no raw credential to the VMM.
Operator-owned sidecars or gateways receive short-lived Instance-scoped authority outside guest memory.

## Cleanup and modules

Release disables ingress first, then removes forwarding, conntrack state, routes, addresses, veth, TAP, namespace, reservations, and ledger ownership.
Every operation is idempotent and reconcile compares durable intent with kernel reality after crashes.

Modules are `intent`, `profile`, `ipam`, `namespace`, `tap`, `firewall`, `dns`, `proxy`, `ingress`, `activate`, `release`, and `reconcile`.
Tests cover IPv4 and IPv6 deny-by-default, spoofing, metadata blocking, DNS bypass, peer isolation, ingress-before-Ready, port races, proxy failure, broker and VMM crashes, ambiguous retries, 100-way assignment, and complete cleanup latency.
