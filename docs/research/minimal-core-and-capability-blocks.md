# The minimal core, and what belongs above it

## The product this serves

SOMA exists to be the smallest sandbox that is still a sandbox, with every other capability plugged
in only when something asks for it. That only means anything if the line between the two is drawn
by a test rather than by taste.

The test used here is deliberately harsh: **if removing it does not stop the machine booting, or
stop the sandbox being usable, it is not core.** Everything that survives that test is named below
with the reason it survives. Everything that fails it becomes a capability a Template opts into.

## The core

**The immutable root block device.** There is nothing to execute without it.

**The vsock device.** It carries the only channel into or out of a sandbox: the authenticated
session, the command, its output, and the shutdown. A machine without it boots and cannot be
reached, which fails the second half of the test.

**The entropy device.** This one looks optional and is not, and the reasoning is worth keeping.
A restored guest wakes holding the snapshot's kernel CSPRNG state, and that state is shared by
every Instance restored from the same Generation. `crates/soma-guest-agent/src/entropy.rs` credits
exactly one contribution, the fresh read from the virtio entropy device, and mixes the launch-page
seed with a credit of zero on purpose: the guest cannot verify where a host-written seed came from,
so crediting it would let untrusted material raise the kernel's entropy estimate and unblock
`getrandom` on a predictable pool. Remove the device and `getrandom` either blocks or answers from
a pool the guest has no reason to trust, and the authenticated session cannot be established.

**The launch page, the authenticated handshake, and the entropy repair.** These are steps rather
than devices, and they fail the removal test for the same reason: without the page an Instance has
no identity and the agent parks at its repair point forever, and without the handshake there is no
session to carry a command.

## The capabilities

**The network device.** A sandbox boots and runs without one. It is currently built on every
launch regardless of the declared policy, including when egress is denied.

**The private writable overlay.** A sandbox that runs one command and exits never writes to it.
The guest agent requires one today because it is written to verify an ext4 superblock, mount an
upper layer, create the upper and work directories, and compose OverlayFS. That is a property of
the agent's code and not of the machine.

**The readiness probe.** It runs a whole command inside the guest before Ready is reported. Removing
it removes a proof, not a capability.

## What each one actually costs, measured

This is the part that changes what to do first, and it contradicts the intuitive answer.

Constructing all five device models takes about **13 microseconds**, and a restored guest never
re-probes a driver at launch because the probe happened once, at capture. So **a sandbox with no
network is not measurably faster**. Making the network a capability is worth doing for surface
area, because every device model parses guest-controlled input and every device is one more thing
inside the snapshot, but it is not a latency change and must not be sold as one.

The overlay is the opposite. Its head clone is the `admitted` to `machine_launched` segment, and
[the speed ladder](../evidence/2026-08-31-speed-ladder.md) measured that segment between **3.7 and
199.5 milliseconds** for the same operation. It is the largest and by far the most variable cost in
the engine, and a read-only sandbox skips it entirely.

So the ordering is:

| Capability | Latency if unused today | Reason to make it optional |
| --- | --- | --- |
| Private writable overlay | 3.7 to 199.5 ms | Latency, and it is the largest cost there is |
| Readiness probe | roughly 3 to 5 ms | Latency |
| Network device | about 0.013 ms | Surface area, not latency |

An unused device is not free even when it is not slow. The always-present network device is what
produced the guest netdev watchdog storm recorded in
[the restore stage timeline](../evidence/2026-08-30-x86_64-restore-stage-timeline.md): the guest
kernel kept firing transmit timeouts against an interface wired to nothing, which then broke the
shutdown acknowledgement.

## What is not negotiable

The roughly twenty-nine millisecond segment between `machine_launched` and `ready` is the same in
every configuration measured, and it is the price of per-Instance cryptographic identity. It could
be very nearly removed by capturing the snapshot after a session exists rather than at the
pre-launch repair point, and that is forbidden: every Instance restored from that image would share
one identity and one key. [ADR 0030](../adr/0030-pre-launch-snapshot-capture-point.md) and
[ADR 0033](../adr/0033-sterile-restored-machine-authority-boundary.md) exist to hold exactly that
line.

If a competitor reports a materially faster time to first command at the same shape, the first
question is not how they optimised the path. It is whether their sandboxes share an identity.

## Consequences

The five-device contract in [the minimal device surface](minimal-device-surface.md) becomes a
maximum rather than a fixed set. A Generation declaring no network builds four devices; one
declaring no writable storage builds three and skips the head clone. A Generation must refuse to
launch against a shape it was not compiled for, in both directions, because a machine that quietly
tolerates a mismatch is a machine whose evidence cannot be trusted.
