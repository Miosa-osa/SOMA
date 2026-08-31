# Restore stage timeline on a development host - 2026-08-30

## Capability status: Live-proved at `c0fd993`, diagnostic only

This record is the first per-stage breakdown of a restored sandbox measured inside the machine rather than at the public receipt.
It exists to say where the time goes, not how fast SOMA is.
**No number here is a latency result and none may be compared with a benchmark figure**, for reasons the host section states.

## Observation identity

| Field | Observed value |
|---|---|
| Host | One developer laptop, not a measurement host |
| CPU | Intel Core Ultra 9 275HX, 24 cores, 24 threads |
| Kernel | Linux 7.0.0-30 |
| Storage | ext4, **no reflink** |
| SOMA revision | `c0fd993` |
| Workload | `node:22`, restored from a prepared snapshot, one sample |
| Command | `soma --format json --backend kvm run node:22 -- /usr/local/bin/node --version` |
| Result | `v22.23.2` returned from the guest; cleanup not proven |

Two properties disqualify this host from producing a latency result.
Its filesystem cannot reflink, so each launch copies the whole ten-gibibyte overlay template instead of cloning it, which was measured at 9.35 GiB of real allocation and seconds of wall clock per launch.
It has 24 threads, so a hundred-way burst would be four times oversubscribed where a measurement host is not.
What the host does support is the internal ordering below, because the machine's own timeline starts after the copy.

## The timeline

Offsets are nanoseconds from the start of machine creation, as recorded by the machine and written by `SOMA_KVM_TIMELINE`.

| Milestone | ms | What completed |
|---|---:|---|
| ValidateManifest | 0.11 | Snapshot identity and compatibility checked |
| CreateVm | 0.39 | The KVM VM exists |
| MapMemory | 0.40 | Captured memory mapped private, no copy |
| RegisterSlots | 0.46 | Certified memory slots registered |
| Platform | 0.53 | Irqchip and PIT |
| Devices | 0.54 | Five device models bound |
| Vcpu | 2.38 | vCPU created with its CPUID template |
| VcpuRestored | 2.44 | Captured vCPU state installed |
| Events | 2.48 | Irqfds, ioeventfds, captured interrupt state |
| LaunchPageWritten | 2.50 | Launch material in its slot |
| EventLoop | 2.55 | Device thread serving |
| RunStart | 2.71 | vCPU 0 entered `KVM_RUN` |
| LaunchPageConsumed | 3.78 | Guest took the material and erased the page |
| VsockConnected | 4.59 | Agent's control connection open |
| Handshake | 13.79 | Authenticated handshake complete |
| LaunchPageRetired | 14.09 | Page verified erased and its slot removed |
| Ready | 16.78 | Fixed readiness probe complete |
| AgentReadyLine | 17.20 | Agent reported ready on its console |
| Execute | 48.73 | `node --version` round trip complete |

## What it shows

**Machine construction is 2.71 ms and all of it precedes the guest.**
Everything up to `RunStart` is work that does not depend on which Instance the machine will serve, which is what makes a prepared worker possible: the [prepared worker protocol](../research/prepared-worker-protocol.md) moves exactly this segment off the request path.
The [eval-1 cohorts](2026-08-31-eval1-burst-and-sequential.md) put the equivalent public milestone at a median of 48.0 ms at concurrency 100, so the same work costs more than an order of magnitude more when a hundred launches contend for it, and none of that cost is arithmetic that had to happen then.

**The two largest steps after the guest resumes are the handshake and the command.**
`VsockConnected` to `Handshake` is 9.2 ms and `AgentReadyLine` to `Execute` is 31.5 ms on this host.
Neither is machine construction, so neither is removed by preparing workers.

## Two defects this run exposed

**A restored guest observes a large time jump.**
`KVM_SET_CLOCK` is applied and `IA32_TSC` is among the restored MSRs, and the guest still sees time that advanced while it was captured: successive restores of one snapshot reported guest uptime of about 190, 429, and 629 seconds.
The netdev watchdog fires on the jump, floods the console, and the shutdown acknowledgement then fails, so cleanup cannot be proven:

```text
virtio_net virtio2 eth0: NETDEV WATCHDOG: CPU: 0: transmit queue 0 timed out 5568 ms
soma-guest-agent: shutdown acknowledgement failed: control Shutdown failed: Io
```

This is why the run above returned `cleanup_incomplete` despite the command succeeding.
It needs a host slow enough to keep a sandbox alive for five seconds to appear, which is why it had not been seen before.

**A filesystem without reflink copies the overlay template per launch.**
The fallback is correct and produces the same private head, and it costs seconds and about ten gibibytes each time.
`setup-host.sh` provisions XFS with reflink, so this affects development hosts rather than provisioned ones, and it is the reason this host cannot produce a latency result.

## Retained artifacts

- [`raw/2026-08-30-restore-stage-timeline/restored-sandbox-timeline.json`](raw/2026-08-30-restore-stage-timeline/restored-sandbox-timeline.json) - the timeline above as the machine wrote it
- [`raw/2026-08-30-restore-stage-timeline/restored-sandbox.console`](raw/2026-08-30-restore-stage-timeline/restored-sandbox.console) - the guest console, including the watchdog and shutdown lines
- [`raw/2026-08-30-restore-stage-timeline/tti-local.sh`](raw/2026-08-30-restore-stage-timeline/tti-local.sh) - the harness used, retained so the boundary it measures is inspectable

## What this record does not prove

- It is one sample on one host and has no distribution.
- It is not a TTI result and must never be compared with a ComputeSDK figure or with any eval-1 number.
- It does not prove cleanup, capacity admission, prepared workers, jailing, networking, or certification.
- The `SOMA_KVM_TIMELINE` output it rests on is a diagnostic with no signature, no identity binding, and no stable schema.
