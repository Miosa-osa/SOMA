# ADR 0038: A published port becomes reachable only at activation and provably stops at release

- Status: Accepted
- Date: 2026-08-31
- Extends: ADR 0028 and the Linux network profile v1

## Context

The broker reserved a host endpoint for every requested publication and installed no forwarding behind it.
`ingress::publish` and `ingress::attach_proxy` returned `Unimplemented` and had no caller, so a caller could ask for a port, receive a port number, and find nothing serving it.
Exposing a service from inside a sandbox is the ordinary reason a coding agent starts a development server, so the gap made the network layer unusable for its most common case.

Two properties had to survive closing it.
Nothing may be reachable before the admitted activation step that opens egress, so a sterile or merely assigned bundle exposes nothing.
Nothing may remain reachable after release, because a port mapping that outlives its Instance eventually points at a stranger's service on a host that reuses ports.

## Decision

Activation installs one `inet` publication table per bundle, named apart from the bundle's host table.
It carries the destination translation at the prerouting hook, so an external client reaches the guest, and at the output hook, so a process on the host reaches it too, and a masquerade out through the bundle's veth so the guest answers a source address that routes back.
Publication is therefore gated by exactly the step that enables forwarding and by the same single-use claimant-bound receipt.

The sandbox and host rulesets are rendered while the bundle is still sterile with the narrow openings a translated connection needs.
Those openings grant nothing on their own: they match only on translation status or on a guest endpoint that nothing routes to, and no translation toward the guest exists until the publication table does.

The guest's answer on a published port is admitted ahead of the protected destination sets.
This is a deliberate narrowing of a fail-closed rule and is confined by four conditions: the spoofing drops above it have already fixed the guest's MAC and source address, the rule names one published endpoint, it requires the reply direction of a conntrack entry, and it requires that entry to be established.
Such an entry can exist only because the inbound rule further down admitted a connection some other party opened.
The alternative was to leave the answer unroutable, because the broker translates every client's source into the bundle's own transit address and every address it could translate to lies inside the private space the protected sets deny.

Release deletes the publication table first and the final live inspection asks for it by name alongside the host table and the host veth, so an incomplete release reports a leaked mapping rather than hiding one.
Reconciliation recognises the same table, so a mapping with no ledger owner is reported.

Admission refuses a publication on an IPv6 host bind, which has no destination to name while the guest lease is IPv4 only.
A reservation socket is bound and never made to listen, because a listening socket completes handshakes the broker cannot forward and makes an unpublished port look reachable.

The layer that turns a host endpoint into a URL is outside this repository and is decided in the published ports and preview URLs research note.

## Consequences

`ingress::publish` now takes the guest address and returns the mapping activation installs, rather than always failing.
`ActivationEvidence` and `ReleaseEvidence` each gain a field naming what was installed and what was removed, which is the whole interface the layer above needs.
A publication costs one additional `nf_tables` transaction at activation and one at release, and nothing at all when an intent publishes no ports.
A loopback publication sets `route_localnet` on the bundle's own veth, which disappears with the link at release rather than persisting as host state.

## Verification gates

- Rendering tests must prove that an unpublished bundle names no guest port and places nothing before the protected drops.
- Rendering tests must prove the host table admits only what its publication table translated, ahead of the drop it excepts.
- A live test in the pinned privileged container must prove the same host endpoint refuses a connection while the bundle is assigned, serves one after activation, and refuses it again after release, with the publication table absent in both refusing states.
