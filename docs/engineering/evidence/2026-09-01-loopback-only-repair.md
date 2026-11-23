# A sandbox with no egress still reaches itself - 2026-09-01

## Capability status: Live-proved at `ac330a1`

[The declared device set](2026-08-31-declared-device-set.md) states that a Generation declaring
the fail-closed isolated policy is built with no network device, and that its guest therefore
"installs no interface identity, address, netmask, route, resolver, or hosts file, because there
is no interface, and raises loopback so that a workload talking to its own address still works."

The first half of that sentence was proved by the device counts in that document. The second half
was not: nothing in it bound a socket. This record binds one.

## Run identity

| | |
| --- | --- |
| Commit | `ac330a1`. The guest agent, the device-set derivation, and the network repair are untouched by the branch this was measured from |
| Host | eval-1, Linux 6.8.0-138-generic, 80 logical CPUs, Intel Xeon Gold 6138 @ 2.00 GHz |
| Image | `busybox:stable-musl`, `linux/amd64` |
| Shape | 1 vCPU, 256 MiB memory, 512 MiB writable storage |
| Path | Prepared restore from a snapshot captured at the pre-launch repair point |
| Samples | 10 sequential launches, each a fresh Instance |
| Retained | [`raw/2026-09-01-loopback-only-repair/`](raw/2026-09-01-loopback-only-repair/) |

The Generation was prepared by `scripts/reproduce.sh` into a directory belonging to this run
only, and the whole tree was removed afterwards. Nothing under `/srv/soma` was reused or
modified.

## What each sandbox was asked

One command, inside the guest, in this order: list `/sys/class/net`, list links and routes, show
`lo`, start a listener on `127.0.0.1:15432` holding one line, connect to it, print what came
back, and show `lo`'s counters again.

## What came back

Identical in all ten. From `lo-0.json`:

```
lo
--net-devices-above--
1: lo: <LOOPBACK,UP,LOWER_UP> mtu 65536 qdisc noqueue state UNKNOWN qlen 1000\    link/loopback 00:00:00:00:00:00 brd 00:00:00:00:00:00
--routes--
--repair--
lo        Link encap:Local Loopback
          inet addr:127.0.0.1  Mask:255.0.0.0
          inet6 addr: ::1/128 Scope:Host
          UP LOOPBACK RUNNING  MTU:65536  Metric:1
soma-lo-ok
--after--
          RX packets:8 errors:0 dropped:0 overruns:0 frame:0
          TX packets:8 errors:0 dropped:0 overruns:0 carrier:0
```

Read in order, that is the whole claim:

- `/sys/class/net` holds **`lo` and nothing else**. There is no `eth0`, so this is a machine
  built with no network device rather than one holding a device with its link down.
- `ip route` printed **nothing at all**. No default route, no gateway, no on-link prefix. Every
  value the full repair would have installed is absent, which is what makes them unmissed.
- `lo` is `UP LOOPBACK RUNNING` and carries `127.0.0.1/8` **before the workload runs**, because
  the guest agent raised it. A fresh Linux guest leaves `lo` administratively down; nothing in
  this command line raised it.
- `soma-lo-ok` crossed. A bind on `127.0.0.1:15432`, a connect to it, and one line delivered
  end to end inside the sandbox.
- The counters moved from `0/0` to 8 packets and 443 bytes each way, so the traffic was real
  rather than short-circuited.

Ten of ten launches exited 0 and printed `soma-lo-ok`.

## What this settles and what it does not

It settles that skipping the network repair wholesale on a declined egress would have been a
regression, and that the loopback-only path this repository ships instead does not have that
defect. Anything binding `127.0.0.1` works in a sandbox that was given no network.

It does **not** measure what the repair costs. The guest's per-step timing is behind the agent's
`timing-report` build and is written to the guest console, which no command-line path surfaces;
reading it needs the instrumented harness [the ready segment split](2026-08-31-eval1-ready-segment-split.md)
used. That document's figure for the **full** repair, on this host, is 3.1 ms, and
[the launch-path audit](../research/launch-path-minimum-viable-audit.md) puts it at 2.65 ms. The
loopback-only path performs two of the full path's ten operations. No figure for it is quoted
here, because none was measured here.

For context and not as a latency result, the ten-sample baseline this Generation was verified
with, at low host load before the cohort above, was 29.14 ms p50 to first command with a `ready`
segment median of 22.1 ms; it is retained as `baseline.txt`.

## The case that must not regress, and why it could not be run

An Instance the broker leased a bundle to must still receive the whole repair: a real MAC,
address, netmask, gateway, resolver, and `/etc/hosts`, all per-Instance. That arm was not run,
and on this host it cannot be, for a reason worth recording.

The device set is derived from the Generation's declared network policy **class**
(`crates/soma-generation/src/generation/template.rs`), and every tool that can prepare a
Generation today compiles the isolated policy: `MachineShape::new` defaults to
`Capabilities::isolated()` and neither `prepare_generation` nor `scripts/prepare-generation.sh`
has an input for anything else. So no reachable command builds a machine with a network device,
and the full repair has no live path at all. This agrees with the ledger, which already records
that the network device "has run only behind the link-down loopback backend".

There is a second, sharper reason the arm matters, recorded in
[ADR 0040](../adr/0040-no-egress-is-the-absence-of-a-device.md): the device set and the launcher
answer "does this Instance get egress" with two different predicates, and for the
`RuntimeDefault` policy class they disagree. Such a Generation is built with a network device and
its guest pays the full repair, installing a gateway and resolver of `10.0.0.1` that route
nowhere. That is the residual of this gap, it is unreachable from any command today, and the ADR
says why it was left alone rather than patched.
