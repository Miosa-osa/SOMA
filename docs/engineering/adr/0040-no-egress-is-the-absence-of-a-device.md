# ADR 0040: No egress is the absence of a device, not a value on the launch page

- Status: Accepted
- Date: 2026-09-01
- Amends: [ADR 0012, fail-closed networking](0012-fail-closed-networking.md), [ADR 0028, declared IPv4 profile for launch network identity](0028-declared-ipv4-profile-for-launch-network-identity.md)

## Context

`network_repair::repair` in the guest agent installs a MAC, an IPv4 address, a netmask, a default
route, a resolver file, and an `/etc/hosts` binding, and it also raises loopback. On a sandbox
that was given no egress, every one of those values was a fiction: the gateway and the resolver
were both `10.0.0.1`, an address `boot::link_down_network` itself supplies precisely because it
routes nowhere, installed on an interface whose frames nothing carried.
[The launch-path audit](../research/launch-path-minimum-viable-audit.md) measured the whole step
at 2.65 ms and classified it removable for a declined egress.

It could not simply be skipped. A fresh Linux guest leaves `lo` administratively down, so a
sandbox whose repair was skipped wholesale could not bind or reach `127.0.0.1`, and a workload
doing something entirely local would break for a reason that has nothing to do with the policy
that denied it the network. The cut had to be loopback-only repair, not no repair.

The blocker was recorded as a wire problem. `LaunchNetwork`
(`crates/soma-guest/src/launch_page/network.rs`) has no representation for "no egress": every
field is mandatory and `LaunchNetwork::new` rejects the zero and unspecified values that would be
the natural spelling for absence, by the declared IPv4 profile ADR 0028 fixed. Adding one looked
like a launch-page schema change, and a launch-page schema change rebuilds every prepared
Generation on every host.

That framing was wrong, and this ADR records the decision that replaced it.

## Decision

**A sandbox with no egress has no network device, and the guest reads that from the machine it
was built as rather than from a value on its launch page.**

The declaration already existed one level up. A Generation's Template declares a network policy;
`TemplateRevision::device_set` turns a policy class of `Isolated` into a `DeviceSet` with no
network slot; the device set decides which device models the VMM builds, which
`virtio_mmio.device=` declarations the kernel command line carries, which manifest sections a
capture writes, and the device-contract digest the snapshot binds. The guest reads its own
command line for `soma.net=` and, finding none, calls `network_repair::repair_loopback_only`,
which raises `lo` and does nothing else. The full repair runs unchanged wherever there is an
interface to repair.

So the answer is carried by the absence of a device rather than by a value describing one.
`LaunchNetwork` is untouched, its declared IPv4 profile still rejects every unroutable value, and
no prepared Generation is invalidated by this reasoning. The device-set derivation itself was
shipped in `ba0cde7` and is
[live-proved](../evidence/2026-08-31-declared-device-set.md); this ADR records why that shape was
the right place for the decision, which was not written down at the time.

Loopback repair is unconditional, and [the live proof](../evidence/2026-09-01-loopback-only-repair.md)
is a sandbox with only `lo` in `/sys/class/net` and no routes at all, binding and connecting to
`127.0.0.1` ten times out of ten.

### What is deliberately dropped

`/etc/resolv.conf` and `/etc/hosts` are not written on this path. `hosts_file` binds the fresh
hostname to the guest's address, and a workload may read it, so this is a product decision rather
than an omission. It is taken deliberately for three reasons: there is no address to bind the
hostname to; the root of a Generation that declared no writable storage is read-only, so both
writes would fail anyway; and `127.0.0.1 localhost`, the only line that would still mean
something, is resolved by every libc's own fallback before a file is consulted. A workload that
needs its own hostname to resolve is a workload that needs a network, which is the policy it was
denied.

## The residual, and why it is not patched here

The device set and the launcher answer "does this Instance get egress" with two different
predicates, evaluated at two different times.

- **Compile time.** `device_set` builds a network device when the declared policy class is
  anything other than `Isolated`.
- **Launch time.** `Egress::claim` (`crates/soma-local/src/backend/kvm/network.rs`) declines
  egress when the request's `EgressPolicy` is `Denied` or `Unspecified`, and otherwise demands a
  broker lease.

For two of the three policy classes these agree. For `RuntimeDefault` they do not:
`NetworkPolicy::runtime_default` carries `EgressPolicy::Unspecified`, so it is not `Isolated` and
gets a device, and it is `Unspecified` so its launch is always `Egress::Declined`. Such a
Generation is built with a network device it can never use, and its guest, seeing `soma.net=` on
its command line, pays the full repair to install the `10.0.0.1` values that route nowhere. That
is exactly the defect this ADR describes, surviving in one policy class.

The obvious patch is to make `device_set` test whether egress is served rather than which class
the policy belongs to. It is rejected. `RuntimeDefault` means "defer to the operator default
profile", and the launcher declining it is a statement about today's launcher, not about the
policy. Deriving a Generation's permanent device contract, and therefore its identity and every
snapshot bound to it, from a behaviour that is expected to change would bake a temporary truth
into an artifact designed to outlive it.

The correct fix is at the launcher: refuse a launch whose request needs egress the Generation was
not built to carry, and whose Generation carries a device the request will never use, rather than
letting the two disagree quietly. That is a compatibility check beside the existing device-
contract check, it is not written, and no command can reach the disagreement today because every
tool that prepares a Generation compiles the isolated policy. It is recorded here so nobody
re-derives it.

## Alternatives considered

### Add a "no egress" representation to `LaunchNetwork`

The original framing: a sentinel or an `Option` on the launch page, which the guest reads to
decide between the two repairs. Rejected. It is a launch-page schema change, so it invalidates
every prepared Generation on every host; it puts a second, weaker copy of a decision the machine
already embodies onto the wire, where the two can disagree; and it would force ADR 0028's IPv4
profile to accept values it exists to reject, or to grow an exception beside it. The device set
answers the same question earlier, once, and in a place a snapshot is already bound to.

### Skip the network repair entirely on a declined egress

The cheapest change and a real regression: `lo` stays down and nothing in the sandbox can bind or
reach `127.0.0.1`. Rejected on that.

### Keep the full repair and accept the cost

2.65 ms, about 11 percent of the guest's own repair path, spent installing values that route
nowhere on every isolated sandbox ever launched. Rejected because the work provably does nothing,
which is a better reason to remove it than the milliseconds.
