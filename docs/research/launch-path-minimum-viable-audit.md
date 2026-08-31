# Is the launch path the minimum viable one? A step by step audit

## Why this exists

The `ready` segment of a receipt has been reported as fixed at about 29 ms on eval-1 across four
configurations that differ in memory and in workload, and it has been described as the cost of
giving one Instance its own cryptographic identity. That description was never checked step by
step. This record walks the launch path in the code, enumerates every step in the order it
happens, measures what each one costs, and says for each one why it is there.

The question is narrow and it is not rhetorical. Nothing may be called the fastest possible until
somebody has walked every step and justified it. A finding that there is nothing left to remove is
a real answer, and part of this record is exactly that finding for most of the path. The rest of
it examines three steps whose justification was not written down anywhere, of which two turn out
to be removable and one turns out to be required after all, and one block of 7.2 ms that no
instrument in the tree can currently attribute.

## What was measured, and on what

Every figure below was taken on eval-1, an Intel Xeon Gold 6138 at 2.00 GHz with 80 logical CPUs,
storage on XFS with reflink, page cache warm after one launch outside every cohort. The code is
this worktree's commit, which adds the prepared machine pool, the exponential wait backoff in the
guest executor, and the ephemeral head clone to the tree the earlier evidence was taken from.

Two instruments were used and they are different in kind.

`SOMA_KVM_TIMELINE` is in the tree already. It writes one JSON file per sandbox at cleanup holding
the machine's own milestone offsets, measured on the host's monotonic clock from the first byte of
the snapshot manifest being read. See `crates/soma-local/src/backend/kvm/timeline.rs`. Nothing had
to be added to use it.

`crates/soma-guest-agent/src/timings.rs` is the guest half. It records twenty two fixed slots, one
per repair step, and renders them as two bounded console lines after readiness has already been
announced, so the measurement sits outside the interval it measures. It is compiled only under the
`timing-report` feature, which the shipped agent does not build with, so reading it needs a guest
agent rebuilt with `SOMA_GUEST_AGENT_FEATURES=timing-report ./scripts/build-guest-agent.sh` and a
Generation prepared and captured with that agent. That was done in a scratch store.

The guest console reaches the host only inside `SandboxEvidence`, and the timeline writer keeps the
console only for a sandbox that failed. To read it for a sandbox that succeeded, the timeline
writer in a throwaway copy of the repository on eval-1 was given four extra lines that write the
same bounded console tail on the success path as `dump_failure` already writes on the failure path.
That patch exists only in the scratch copy, it is a measurement instrument rather than a proposed
change, and no crate in this worktree was touched.

### Reproduction

```sh
SOMA_GUEST_AGENT_FEATURES=timing-report ./scripts/build-guest-agent.sh
scripts/prepare-generation.sh busybox:stable-musl <store> <fs-tools> 1024 10240
target/release/examples/capture_snapshot <store>/ref-<digest> 1024
SOMA_HEAD_DIR=<heads> SOMA_ALLOW_UNCERTIFIED_GENERATION=1 \
SOMA_GENERATION_STORE=<store> SOMA_KVM_TIMELINE=<timelines> \
  target/release/soma --format json --backend kvm run \
    --memory-mib 1024 --storage-mib 10240 busybox:stable-musl -- /bin/busybox --help
```

A Generation restores only at the memory it was captured with, so `--memory-mib` and
`--storage-mib` must match the capture or the launch is refused before a machine exists.

### Sample sets

| Set | What | n | Note |
| --- | --- | ---: | --- |
| A | Shared store, shipped agent, receipts and machine timelines | 20 | Host load average about 1.9 |
| B | Scratch store, instrumented agent, timelines and guest console | 20 | Host load average about 12 |
| C | Scratch store, instrumented agent, timelines and guest console | 30 | Host load average about 22 |
| D | Shared stores at 128, 512 and 1024 MiB, shipped agent | 12 each | Host load average about 2.6 |

Sets B and C agree with each other to within 0.15 ms on every segment despite a factor of two in
host load. Set A was taken with the shipped agent against a separately captured Generation of the
same image and shape, and its total from arming the vCPU to ready is 24.77 ms against set C's
23.71 ms. It agrees with set C on the two later segments and disagrees on where exactly the
boundary between the first two falls; that disagreement is discussed where it matters, below. The
busybox workload is the control: it does almost nothing when it starts, so what these numbers
measure is the engine rather than the program.

## The outer frame

Receipt milestones, set A, medians in milliseconds.

| Milestone | Offset | Delta | What the delta is |
| --- | ---: | ---: | --- |
| accepted | 0.00 | 0.00 | |
| workload_resolved | 0.00 | 0.00 | The store is read before the request is timed |
| admitted | 0.04 | 0.04 | Request validation |
| machine_launched | 6.68 | 6.64 | Opening the artifacts and cloning the private head |
| ready | 36.45 | 29.55 | Everything this record is about |
| command_finished | 40.13 | 3.24 | `busybox --help` |

The head clone segment ranges from 3.25 ms to 21.58 ms across twenty launches of one
configuration. That variance is being measured elsewhere and is not this record's subject.

The `ready` segment of 29.55 ms decomposes, in the same set, into 3.16 ms of restore before the
vCPU is armed, 24.77 ms between arming the vCPU and the machine being marked ready, and 1.03 ms
that has no milestone of its own and is bounded by the difference between the receipt segment and
the machine's internal `Ready` offset of 28.52 ms. That remainder is the sandbox thread being
spawned, one `HostLaunchMaterial::generate` and two channel sends. The instrumented Generation of
sets B and C puts the same three parts at 3.58 ms, 23.71 ms and the same unmilestoned remainder.

A point that matters for reading everything below: all twenty receipts of set A, which is the only
set whose receipts were retained, report `preparation: on_demand`, and the code makes that
structural rather than incidental. The prepared machine pool defaults to a target of one
(`crates/soma-local/src/backend/kvm.rs:42`) and its replenisher runs on its own thread, but
`soma run` is one process per sandbox, so the pool is always empty when the only Launch of that
process arrives. On this topology the prepared arm at `start.rs:67` is unreachable and the 3.58 ms
restore is on the request path by construction.

## The step table

Files are relative to `crates/`. Costs are medians unless a range is given. A step with no
instrument of its own carries the bound its neighbours give it, and says so.

### Host, before any machine exists

| # | Step | Where | Cost | Class |
| ---: | --- | --- | ---: | --- |
| 1 | Stamp the admitted milestone | `soma-local/src/backend/kvm/lifecycle.rs:51` | below 10 us | REQUIRED |
| 2 | Refuse a second live sandbox in this Backend | `lifecycle.rs:55` | below 1 us | REQUIRED |
| 3 | Refuse a shape that is not one vCPU | `lifecycle.rs:61` | below 1 us | REQUIRED |
| 4 | Downcast the prepared Generation | `lifecycle.rs:64` | below 1 us | REQUIRED |
| 5 | Derive Instance bytes, a fresh operation, and the guest CID | `lifecycle.rs:68`, `identity.rs:31` | below 100 us, bounded by step 6 | SECURITY |
| 6 | Claim egress, which returns `Declined` when the policy denies it | `lifecycle.rs:73`, `network.rs:46` | below 1 us on this path | SECURITY |
| 7 | Build the link down launch network, including a wall clock sample | `lifecycle.rs:76`, `boot.rs:224` | below 10 us | SECURITY |
| 8 | Register the Generation with the pool and try to claim a machine | `lifecycle.rs:87`, `claim.rs:110` | 18.4 us, previously measured | REQUIRED |
| 9 | Open kernel, initramfs and root artifacts from the store | `start.rs:116`, `boot.rs:34` | part of the 6.64 ms below | REQUIRED |
| 10 | Reflink the private overlay head from the snapshot template and unlink it | `boot.rs:100`, `boot.rs:131` | 6.64 ms for steps 9 and 10 together, range 3.25 to 21.58 | SECURITY |
| 11 | Register the Instance with the Host Runtime | `start.rs:118` | below 100 us, bounded by the 1.0 ms remainder | SECURITY |
| 12 | Stamp the machine launched milestone | `start.rs:119` | below 10 us | REQUIRED |
| 13 | Spawn the sandbox thread and wait for its answer | `session.rs:169` | part of the 1.0 ms remainder | REQUIRED |
| 14 | Generate the launch material: nonce, Instance PSK, guest entropy, responder keypair | `worker.rs:59`, `soma-guest/src/launch_page.rs:86` | part of the 1.0 ms remainder | SECURITY |

The 1.0 ms remainder covering steps 11, 13 and 14 is the difference between the receipt's
`machine_launched` to `ready` segment of 29.55 ms and the machine's own `Ready` offset of
28.52 ms in set A. None of the three has an instrument of its own.

### Host, restoring the machine

Set C, medians, deltas in milliseconds from the previous milestone. This is the 3.58 ms that
precedes the vCPU being armed.

| # | Step | Where | Cost | Class |
| ---: | --- | --- | ---: | --- |
| 15 | Read the state object, decode the manifest, check host compatibility, read the repair point marker | `soma-kvm/src/x86_64/snapshot/restore.rs:186` | 0.61 | REQUIRED |
| 16 | Create the VM | `restore.rs:199` | 0.45 | REQUIRED |
| 17 | Map the memory object privately and adopt the mapping | `restore.rs:206` | 0.04 | SECURITY |
| 18 | Map and register the launch page slot before the vCPU exists | `restore.rs:225` | 0.08 | SECURITY |
| 19 | Register the certified memory slots | `restore.rs:229` | 0.31 | REQUIRED |
| 20 | Recreate the irqchip, PIT and interrupt routes | `restore.rs:235` | 0.43 | REQUIRED |
| 21 | Recreate the five device models | `restore.rs:237` | 0.05 | REQUIRED |
| 22 | Create the vCPU | `restore.rs:247` | 0.26 | REQUIRED |
| 23 | Write CPUID, MSRs and the captured register state | `restore.rs:253` | 0.17 | REQUIRED |
| 24 | Register the serial irqfd, the IRQ lines and the notify fds, then arm the captured irqchip, PIT and clock | `restore.rs:259` | 0.45 | REQUIRED |
| 25 | Write the fresh launch page into its slot | `restore.rs:114` | 0.07 | SECURITY |
| 26 | Spawn the device event loop thread | `soma-kvm/src/x86_64/sandbox.rs:179` | 0.27 | REQUIRED |
| 27 | Install the vCPU run mask, spawn the vCPU thread, wait for it to report ready | `sandbox.rs:205`, `soma-kvm/src/x86_64/watchdog.rs:122` | 0.34 | REQUIRED |

Step 18 is placed before the vCPU exists on purpose, and the comment at `restore.rs:219` records
why: the identical `KVM_SET_USER_MEMORY_REGION` after the vCPU exists costs two milliseconds. The
measured cost here is 0.08 ms, so that ordering is worth about 1.9 ms and is already taken.

Nothing in steps 15 to 27 is removable. Every one of them is a kernel call whose result the next
one needs, and the whole block is 3.58 ms, which is 12 percent of the `ready` segment. It is also
the block that the prepared machine pool exists to move off the request path, and on a topology
where the pool can be warm it costs the request nothing at all.

### The ready segment, from arming the vCPU to the machine being ready

This is the segment the question is really about. Set C, thirty samples, medians in milliseconds.
The guest column is the sum of the guest agent's own slots that fall inside the segment.

| Segment | Host | Guest accounted | Remainder |
| --- | ---: | ---: | ---: |
| Arm vCPU to launch page observed erased | 7.23 | 0.02 | 7.21 |
| Launch page erased to vsock connected | 2.36 | 2.55 | -0.17 |
| Vsock connected to handshake complete | 8.04 | 7.44 | 0.71 |
| Handshake complete to ready | 6.68 | 6.15 | 0.53 |
| **Total** | **23.71** | **16.16** | **7.55** |

The two negative or small remainders are boundary effects rather than error: the guest begins the
readiness probe immediately after sending `RepairComplete`, while the host is still verifying the
erased page and deleting its memory slot, so the two overlap. Taking the last two segments together
removes the overlap and leaves 0.53 ms of genuine host side work.

Set B, taken at half the host load, reports 7.20, 2.46, 7.91 and 6.57 for the same four segments.
The split is reproducible.

### Guest, per step

Set C, thirty samples, microseconds. These are the twenty two fixed slots of
`soma-guest-agent/src/timings.rs`, in the order the agent reaches them.

| Slot | What it is | Where | p50 | p99 | min | max | Class |
| --- | --- | --- | ---: | ---: | ---: | ---: | --- |
| `wake` | The launch page poll sleep the restore interrupted | `launch_page.rs:207` | 0 | 0 | 0 | 0 | see below |
| `look` | The sixteen byte domain probe that found the page | `launch_page.rs:103` | 7 | 17 | 5 | 17 | REQUIRED |
| `copy` | Copying 4096 bytes out of the `/dev/mem` view into locked memory | `launch_page.rs:74` | 4 | 229 | 3 | 229 | SECURITY |
| `erase` | Overwriting the view with zeroes and reading every byte back | `launch_page.rs:75` | 6 | 6 | 3 | 6 | SECURITY |
| `parse` | Validating and parsing the locked copy | `launch_page.rs:76` | 386 | 693 | 242 | 693 | SECURITY |
| `hwrng` | One 64 byte read from the virtio entropy device | `entropy.rs:74` | 330 | 2369 | 190 | 2369 | SECURITY |
| `mix` | Two `RNDADDENTROPY` calls plus `RNDRESEEDCRNG` | `entropy.rs:78` | 54 | 192 | 29 | 192 | SECURITY |
| `crng` | Proving `getrandom` no longer blocks | `entropy.rs:79` | 34 | 220 | 19 | 220 | SECURITY |
| `cid` | Waiting for the vsock device to report the assigned identifier | `control.rs:177` | 158 | 240 | 87 | 240 | SECURITY |
| `vsock` | Creating and connecting the control socket | `control.rs:215` | 1479 | 1956 | 915 | 1956 | REQUIRED |
| `ident` | Identity repair | `identity.rs:75` | 3734 | 6393 | 2834 | 6393 | SECURITY |
| `net` | Network repair | `network_repair.rs:72` | 2646 | 4083 | 2281 | 4083 | see below |
| `hswait` | Handshake time blocked reading the host's first message | `main.rs:165` | 33 | 72 | 24 | 72 | REQUIRED |
| `hssend` | Handshake time writing the second message | `main.rs:165` | 18 | 28 | 13 | 28 | REQUIRED |
| `hswork` | Handshake time that is not transport, which is the Noise work | `main.rs:165` | 598 | 1071 | 432 | 1071 | SECURITY |
| `req` | Blocked waiting for `PrepareAndProbe` | `lifecycle.rs:120` | 386 | 562 | 255 | 562 | REQUIRED |
| `report` | Sending `RepairComplete` | `lifecycle.rs:127` | 57 | 2656 | 11 | 2656 | REQUIRED |
| `spawn` | `fork` plus `execve` of the probe | `executor.rs:99` | 3644 | 5277 | 1427 | 5277 | SECURITY |
| `stream` | Polling the probe's two pipes to end of file | `executor.rs:117` | 317 | 1301 | 14 | 1301 | REQUIRED |
| `wait` | Waiting for the exit status after the pipes closed | `executor.rs:127` | 748 | 981 | 18 | 981 | REQUIRED |
| `reap` | Killing, reaping and sweeping every descendant | `executor.rs:130` | 986 | 3038 | 889 | 3038 | see below |
| `term` | Sending the terminal report | `lifecycle.rs:109` | 89 | 186 | 70 | 186 | REQUIRED |
| | **Sum over the whole line** | | **15886** | **24225** | **14685** | **24225** | |

The guest steps that run before the capture point are not on the launch path at all and are
therefore absent from this table. `boot::early_init` at `boot.rs:83`, `warm::runtime` at
`warm.rs:58` and `pid1::sync` at `main.rs:133` are paid once per Generation, by the builder, and
every restored Instance inherits their result. That is the whole point of ADR 0030's capture point,
and it is why the launch path contains no mount, no superblock verification and no pivot.

## Three steps that had to be justified from scratch

### Network repair on a sandbox that has no network, 2.65 ms

The measured configuration asks for no egress. `Egress::claim` at `network.rs:52` returns
`Declined`, `attachment` returns `None` at `network.rs:80`, `pending_activation` returns `None` at
`network.rs:87`, and `open_network` at `worker/activation.rs:26` therefore returns immediately
without ever sending `Request::RaiseLink`. The machine's link gate is never raised. The comment at
`boot.rs:214` states the position plainly: the device exists so the guest's repair step has one to
configure, and no packet leaves the machine.

Inside that machine the guest then spends 2.65 ms in `network_repair::repair`. It opens an
`AF_INET` socket, reads and clears `IFF_UP`, installs a MAC, installs an address, installs a
netmask, raises loopback, raises `eth0`, adds a default route through 10.0.0.1, and writes
`/etc/resolv.conf` and `/etc/hosts`. One of those steps is provably a no op in value terms: the MAC
it installs is `02:53:4f:4d:41:01` from `boot.rs:228`, that is the only MAC constant in the whole
tree, and it is also the MAC the snapshot captured and `restore.rs:233` restores. The rest install
values that cannot carry a packet, because the route and the resolver both point at 10.0.0.1, an
address `boot.rs:216` describes as routing nowhere, on an interface whose link gate is down.

ADR 0023 requires the agent to install a fresh MAC, address, prefix, gateway and resolver **before
it raises the link**. On this configuration the link is never raised, so the precondition the
requirement attaches to does not occur. ADR 0012 requires that a machine which asked for no network
gets no network, and skipping the repair would satisfy that more directly than performing it does.

Classification: **REMOVABLE for a declined egress**, at 2.65 ms, which is 11 percent of the
23.71 ms between arming the vCPU and ready and 9 percent of the receipt's 29.55 ms segment. Two caveats belong with it rather than after it. The `/etc/hosts` line binds the fresh
hostname to the guest's address and a workload may read it, so that write is a product behaviour
and would have to be kept or consciously dropped. And the whole step must stay exactly as it is for
an Instance the broker leased a bundle to, where every one of those values is real and
per-Instance. What is unjustified is paying it on the path where none of them are.

### The readiness probe sweeps a group that is provably empty, 0.99 ms

After the fixed self probe exits, `executor.rs:130` kills the process group, reaps the group, reaps
orphans, and sweeps `/proc` for strays. That costs 0.99 ms at p50 and 3.04 ms at p99.

The probe is `/proc/self/exe --soma-ready-probe-v1` with a one second timeout and a one byte output
allowance, fixed at `soma-guest-agent/src/lifecycle.rs:46`. Its whole body is the early return at
`main.rs:63`. It cannot fork, so the group it leads cannot contain anything but itself, and the
`/proc` sweep cannot find a stray it created.

Classification: **REQUIRED, by the wording of ADR 0003 rather than by mechanism**. ADR 0003 defines
readiness as a real command completed through the production executor, and an executor that takes a
shortcut for one command is no longer the production executor for it. This is a genuine tension
between a security definition and a millisecond, it is currently resolved in favour of the
definition, and it is worth recording that the resolution costs 0.99 ms rather than nothing.

### The entropy read has a two millisecond first poll, 2.0 ms on the tail

`entropy.rs:40` sets `POLL` to two milliseconds and `read_hardware` at `entropy.rs:94` sleeps for a
full interval whenever `/dev/hwrng` is not immediately ready. The `hwrng` slot shows this directly:
330 us at p50 and 2369 us at p99, and the tail is one poll interval wide rather than being a
gradual distribution. The same shape appears in `cid`, whose `CID_POLL` at `control.rs:26` is also
two milliseconds, though on this host the first read always succeeds.

The pattern has already been fixed once elsewhere in the tree. `executor.rs:38` and `executor.rs:187`
replaced a flat 5 ms wait with a sequence that starts at 50 us and doubles to a 5 ms ceiling,
because a flat first interval was the whole of the readiness probe whenever the parent lost the
race. The earlier evidence records that flat wait costing 5.16 ms at p50 on a developer laptop; on
eval-1 with the backoff in place the same `wait` slot is 748 us.

Classification: **REMOVABLE tail**, about 2.0 ms on the launches that pay it, roughly one in thirty
here. Nothing justifies the first poll being two milliseconds rather than tens of microseconds, and
the fix already exists in the same crate.

## The 7.2 ms nobody can currently attribute

The largest single item in the ready segment is the 7.23 ms between the host arming the vCPU and
the host observing that the guest has erased the launch page. It is 30 percent of the 23.71 ms
between arming the vCPU and ready, and 24 percent of the receipt's 29.55 ms segment. The guest's
own clock accounts for 17 us of it, being `look` plus `copy` plus `erase`.

What is known about it:

- It is not host poll granularity. `CONSUME_POLL` at `soma-kvm/src/x86_64/sandbox/launch.rs:20`
  is 100 us, so at most 100 us of the interval is the host not having looked yet.
- It is not guest polling. `PAGE_POLL` at `soma-guest-agent/src/main.rs:103` is 100 us, and the
  host writes the page before it resumes the vCPU, so the page is already present at the first
  check.
- It is not scheduling noise. Over thirty samples the interval ranges from 6.44 ms to 7.60 ms,
  which is the tightest spread of the four segments of the ready block. Set B, taken at roughly
  half the host load, reports 7.20 ms.
- It does not scale with guest memory. Set D measures 6.87 ms at 128 MiB, 7.07 ms at 512 MiB and
  5.56 ms at 1024 MiB on the shipped agent, so the interval is not proportional to the mapped
  memory object and the largest machine is the fastest of the three.
- Its boundary with the next segment moves between Generations, but the pair does not. Set A
  measures 5.62 ms here and 3.78 ms for the next segment; set C measures 7.23 ms and 2.36 ms. The
  two together are 9.40 ms in set A and 9.59 ms in set C. The host therefore marks the erased page
  at a different point in the two Generations, while the block from arming the vCPU to the vsock
  being connected is 9.5 ms either way and the guest accounts for 2.4 to 2.6 ms of it.
- It is not host page faulting. A complete sandbox run costs the whole process 1381, 1464 and 1380
  minor faults and no major faults across three runs, with a peak resident set of 18 MB. Faulting
  the private mapping in cannot account for seven milliseconds at that count.
- The guest cannot see it. The `wake` slot reads exactly 0 in all fifty samples of sets B and C,
  for a sleep that asked for 100 us. That is only possible if the guest's monotonic clock does not
  advance across the resume, which means guest side durations taken in that window understate real
  time and the guest's own instrumentation is blind to this interval. From the next slot onwards
  the guest clock agrees with the host to within 0.2 ms per segment, so the blindness is confined
  to this window.

Classification: **UNKNOWN**. It is not possible to say from the existing instruments whether this
is the first `KVM_RUN` entry on a restored vCPU, the guest kernel resuming its timekeeping and its
timers after a restored `KVM_SET_CLOCK`, the guest sitting halted until a restored local APIC
deadline fires, or something else. Every one of those would produce a tight, memory independent,
load independent interval that the guest clock cannot see, which is exactly what was measured.

The instrument that would settle it does not exist yet and would have to be added to a crate this
audit does not own: a milestone taken inside the vCPU worker immediately before and immediately
after the first `KVM_RUN` returns, and a count of exits by reason over the first ten milliseconds.
That is the single most valuable measurement left on this path, because it is worth more than
everything classified removable in this record put together.

## What is genuinely minimal

Most of the path is. Setting out the working rather than asserting it:

The restore, steps 15 to 27, is 3.58 ms of kernel calls in a fixed order where each one's result
is the next one's input. Building all five device models costs 0.05 ms, so removing the network
device would not measurably help and is a surface area decision rather than a speed one. The launch
page slot is already registered in the one position where it costs 0.08 ms instead of 2 ms. The
whole block is what the prepared pool exists to remove from the request path, and ADR 0033 already
draws the line that lets it be removed without a machine holding another tenant's authority.

The launch page handling is 403 us in total: 7 us to find it, 4 us to copy it into locked memory,
6 us to overwrite the view and read every byte back, and 386 us to validate and parse the locked
copy. The suspicion that byte at a time volatile access through `/dev/mem` would cost milliseconds
is refuted by the copy and erase figures. The parse is the expensive part and it is the part that
checks the page is well formed before anything trusts it.

The Noise handshake is 649 us of which 598 us is responder work on the guest, 33 us is blocked
reading and 18 us is writing. The peer is never the reason the guest waits. There is no round trip
to remove and no cryptography to cheapen without changing what the session proves.

Entropy repair is 418 us at p50 excluding the poll tail discussed above, and it is what stops every
Instance restored from one snapshot sharing the captured kernel CSPRNG state.

Identity repair is 3.73 ms and is the largest single guest step. It is two hostname writes, an
atomic `machine-id` replacement that forces an overlay copy up, two tmpfs mounts over `/run` and
`/tmp`, and one `clock_settime`. ADR 0003 names duplicate machine identity, stale session state and
invalid clocks as the exact hazards that make a restored guest unsafe to expose. There is no
instrument that splits those five operations, so which of them dominates the 3.73 ms is a smaller
UNKNOWN inside a step that is not in question.

The probe spawn is 3.64 ms and is the second largest guest step. It is one `fork` from PID 1 and
one `execve` of a statically linked musl binary that returns from `main` immediately. Removing it
means abandoning ADR 0003's definition of ready, which is the same trade the pre launch capture
point already refuses.

## What would actually be worth doing, in order

1. Instrument the first `KVM_RUN` on a restored vCPU. Worth up to 7.2 ms, which is 30 percent of
   the arming to ready block, and currently invisible to both halves of the instrumentation.
2. Skip network repair when the Instance was given no egress. Worth 2.65 ms, 11 percent of the
   same block, with the `/etc/hosts` write to decide about separately.
3. Give the entropy and CID polls the same first interval and backoff the executor's wait already
   has. Worth about 2 ms on the launches that hit the tail.
4. Split the identity repair step into its five operations, so that 3.73 ms stops being one number.
5. Make the prepared arm reachable on the measured topology. Worth the whole 3.58 ms restore, and
   it is a topology change rather than a code change: the arm exists and is exercised, but one
   process per sandbox can never find a warm pool.

Items one and two are 9.9 ms of a 29.55 ms segment at the median, item three is a further 2 ms on
the launches that hit its tail, and only the second of the three is understood today. Until the
first item is measured, no claim that this path is the minimum viable one can be supported, and
this record does not make one.

## What this does not prove

- One host, one commit, one workload, and four sample sets. The busybox control says what the
  engine costs; it says nothing about a `node:22` figure beyond what the earlier evidence already
  records.
- Concurrency one only. Nothing here says how any of these steps behaves in a hundred way burst,
  and the earlier evidence shows that the ready segment roughly doubles at that concurrency.
- No network, no jail, no certified Generation, no prepared machine. Every removability claim
  about network repair is scoped to the declined egress path and is explicitly not made for a
  leased one.
- The guest side figures come from an agent built with `timing-report`, which is 835,760 bytes;
  the earlier evidence records an uninstrumented build of 757,936 bytes on a different commit, and
  no uninstrumented build of this commit was measured. The instrumented Generation and the shipped
  one agree on the block from arming the vCPU to the vsock being connected and on both later
  segments, and disagree by 1.6 ms on where inside that block the host observes the erased page.
  That disagreement is not explained here.
- The timeline files are diagnostics, not evidence. They carry no signature, no identity binding
  and no stable schema, and nothing here should be quoted as a measurement of record.
