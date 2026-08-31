# The ready segment on eval-1, split and shortened - 2026-08-31

## Capability status: Live-proved at `a79ac1e`, diagnostic split

The `machine_launched` to `ready` segment of the receipt is 27.9 ms on the measurement host and does not move with memory or with workload.
This record is the first per-stage split of it measured on that host rather than on a laptop, and it reports what one change to that split removed.
The split itself comes from `SOMA_KVM_TIMELINE` and the guest agent's `timing-report` build, both of which are diagnostics: **no number in the split sections is a latency result** and none may be quoted as one.
The before-and-after totals in the last section are receipt milestones from matched cohorts and may be compared with each other, and with nothing else.

## Observation identity

| Field | Observed value |
|---|---|
| Host | eval-1, one bare-metal Ubuntu host |
| CPU | Intel Xeon Gold 6138 at 2.00 GHz, 80 logical CPUs |
| Storage | XFS on `/srv` with `reflink=1` |
| Workload | `busybox:stable-musl`, 128 MiB of memory, 1024 MiB of storage |
| Command | `/bin/busybox --help` |
| Path | Prepared restore from a snapshot taken at the pre-launch repair point |
| Concurrency | One. This is the sequential case, where the fixed cost is the whole cost |
| Samples | 25 per cohort, three interleaved rounds per arm, medians reported |

Both arms were prepared and captured from their own tree with the same script, so the guest agent in each snapshot is the one its host binary speaks to.
The two arms were run alternately in the same session, so host drift is shared.

## Where the 27.9 ms went

Offsets are nanoseconds from the start of machine creation as the machine itself recorded them, so `RunStart` is where the restored guest resumes and everything after it is the segment in question.

| Step | ms | What it is |
|---|---:|---|
| `RunStart` to `LaunchPageConsumed` | 7.16 | The restored guest before it reaches its own code |
| to `VsockConnected` | 2.23 | Entropy repair, then the control socket |
| to `Handshake` | 7.64 | Identity repair, network repair, then two Noise messages |
| to `LaunchPageRetired` | 3.91 | The repair round trip and the launch page's memory-slot removal |
| to `Ready` | 2.22 | The fixed readiness self-probe |

The guest agent's own timing report, from a `timing-report` build of the same Generation, attributes the guest half of that (microseconds, median of 25):

```text
wake=6347 look=7 copy=5 erase=6 parse=590 hwrng=1060 mix=84 crng=27 cid=154 vsock=1554 ident=3882
net=3119 hswait=33 hssend=19 hswork=626 req=521 report=61 spawn=4354 stream=232 wait=735 reap=890
```

Read against the host steps, almost the whole segment is the guest: identity repair is 3.9 ms, network repair is 3.1 ms, opening the vsock socket is 1.6 ms, the Noise work is 0.6 ms, and forking and reaping the readiness probe is 6.2 ms in this instrumented build.

## Three findings the split produced

**The launch-page poll is not a poll problem.**
`wake` is the length of the 100-microsecond sleep the restore interrupted, and it reads 6.35 ms.
Replacing that sleep with a spin, so the guest is never asleep at the capture point, drove `wake` to **zero** and moved the host's `RunStart` to `LaunchPageConsumed` step not at all: 6.98 ms against 7.05 ms on the same tree.
So the 7 ms is spent before the guest executes its next instruction, and the guest's own monotonic clock cannot see it.
A host-side trace of the first forty `KVM_RUN` exits shows why: after two serial-port exits and two virtio interrupt acknowledgements, the vCPU stays inside one `KVM_RUN` call for 4.7 ms without a single userspace exit, which is guest-kernel resume work and demand paging of the private memory mapping, invisible from both ends.
It is now the largest item in the segment and nothing in this change touches it.

**The host's ephemeral keypair is not on the critical path.**
`start_initiator`, which is the X25519 keygen plus the Noise setup and the first message, was measured at 0.42 ms over eight runs.
The host completes it and writes handshake message one about 7 ms before the guest, still repairing its identity and network, reads it: the guest's own `hswait` is 33 microseconds, meaning the message was already waiting.
Generating it earlier would remove 0.42 ms from a window that already has 7 ms of slack, so it would remove nothing.

**Retiring the launch page early is slower, not faster.**
Removing the page's KVM memory slot costs a read-side grace period.
Moving that removal from the repair commit to just after the guest's control connection opens, so it overlaps identity and network repair, was implemented and measured on the same Generation: the ready segment went from 22.6 and 22.9 ms to 23.9 and 24.2 ms.
Disturbing a running guest with a memory-slot removal costs more than the overlap saves, so the removal stays where it was.

## What was removed, and what it was worth

The readiness probe was removed: `PrepareAndProbe` became `Prepare` with no body, and Ready is now the authenticated repair report alone, as [ADR 0039](../adr/0039-repair-report-alone-proves-readiness.md) records.

Receipt segments, medians of 25 samples, three interleaved rounds per arm:

| Round | before, `machine_launched` to `ready` | after |
|---|---:|---:|
| 1 | 28.06 ms | 22.60 ms |
| 2 | 27.05 ms | 22.81 ms |
| 3 | 27.59 ms | 22.48 ms |
| median | **27.59 ms** | **22.60 ms** |

That is 4.99 ms, or 18.1 percent of the segment, off every Instance at every concurrency.

Those three rounds were run on the tree the change was developed on.
The published revision was rebuilt afterwards and re-measured: its guest agent has the same digest and its command line is byte-identical, and one further pair of cohorts on a host that was busier by then read 28.04 ms and 24.23 ms.
The gap is what carries; the absolute figures move with whatever else the host is doing.

The saving is the same size in a different shape.
`node:22` at 1024 MiB of memory and 10240 MiB of storage, twenty samples per cohort, two interleaved rounds, running `/usr/local/bin/node --version`:

| Round | before | after |
|---|---:|---:|
| 1 | 30.16 ms | 25.33 ms |
| 2 | 30.31 ms | 25.05 ms |

That is 5.05 ms against busybox's 4.99 ms at an eighth of the memory and a tenth of the storage, which is what a fixed cost being removed looks like.
The absolute figures are higher than the busybox pair because the host was busier and because this arm's before Generation is the older prepared one; only the gap within a pair carries.

The internal split moved in two places rather than one:

| Step | before | after |
|---|---:|---:|
| `RunStart` to `LaunchPageConsumed` | 7.16 | 6.93 |
| to `VsockConnected` | 2.23 | 2.25 |
| to `Handshake` | 7.64 | 7.91 |
| to `LaunchPageRetired` | 3.91 | 1.46 |
| to `Ready` | 2.22 | 0.02 |

The probe's own 2.20 ms is the obvious half.
The other 2.45 ms is the memory-slot removal getting cheaper: it used to run while the single vCPU was forking and reaping the probe, and now it runs against an idle guest.

## What could not be removed

Identity repair, 3.9 ms, and network repair, 3.1 ms, are 7 of the remaining 22.6 ms.
They write this Instance's hostname and machine identity, replace its session directories, set its clock, and install its network identity, and they run after the resume precisely because the snapshot is captured before any of them exists.
Capturing later, or sharing any of it between Instances, would remove them and would be the exact violation [ADR 0030](../adr/0030-pre-launch-snapshot-capture-point.md) and [ADR 0033](../adr/0033-sterile-restored-machine-authority-boundary.md) exist to prevent.
They stay.

## Reproducing

```bash
export SOMA_GENERATION_STORE=<prepared store> SOMA_HEAD_DIR=<head dir>
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1 SOMA_KVM_TIMELINE=<empty directory>
soma --format json --backend kvm run --memory-mib 128 --storage-mib 1024 \
    busybox:stable-musl -- /bin/busybox --help
```

Each sandbox writes one JSON file of milestone offsets into the timeline directory.
For the guest half, rebuild the agent with `SOMA_GUEST_AGENT_FEATURES=timing-report ./scripts/build-guest-agent.sh`, prepare and capture a Generation with it, and read the two `timing` lines from the guest console.
