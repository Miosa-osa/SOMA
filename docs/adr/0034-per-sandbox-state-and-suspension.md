# ADR 0034: Per-sandbox suspension is a tenant object, separate from the Generation snapshot

- Status: Accepted
- Date: 2026-08-31
- Extends: ADR 0002, ADR 0010, ADR 0012, ADR 0030, ADR 0031, ADR 0032, and ADR 0033

## Context

Every provider in [the capability survey](../research/sandbox-provider-capability-survey.md) offers two things SOMA does not have: a sandbox that can be paused and resumed, and a snapshot taken after the user has changed something.
E2B preserves filesystem and memory and documents roughly four seconds per gibibyte to pause and about one second to resume, Vercel persists by default and auto saves on stop, Modal documents an `idle_timeout` whose activity is commands, standard input, or live TCP, and E2B, Vercel, and Modal all document snapshotting a running sandbox so that a later run can skip dependency installation.
[The gap map](../research/gap-map.md) records the same gap in section 5 and says the honest thing about it: this is not unimplemented work, it is an undesigned object.

SOMA's snapshot is a build-time artifact.
[ADR 0030](0030-pre-launch-snapshot-capture-point.md) fixes its capture point at the disconnected repair wait the pinned guest agent enters before any launch page is written, precisely so that the captured memory never held an Instance identity, a session key, a context identifier, or network identity, because none of those values had been created yet.
[ADR 0032](0032-candidate-bound-snapshot-identity.md) binds that snapshot to the Candidate whose immutable artifacts were booted, and [ADR 0033](0033-sterile-restored-machine-authority-boundary.md) lets a restored machine exist before any Instance does, on the strength of the same property: a snapshot contains no reusable Instance authority, so every Instance of a Generation may share one.

A per-sandbox object inverts every one of those properties.
It is captured after a tenant has run commands, so it contains tenant memory and a tenant disk, and it may contain a credential that was delivered into the guest.
It belongs to one tenant rather than to a Generation, it is written at an arbitrary moment rather than at a certified point in a build pipeline, and it outlives the Instance that produced it rather than preceding every Instance.
Sharing it is a data breach where sharing the Generation snapshot is the entire point.

Treating the two as one kind of object, with a flag distinguishing them, would be the mistake.
The same restore code path would then be reachable with tenant bytes in it, the prepared worker pool would be one type confusion away from handing a tenant's memory to another tenant, and the property ADR 0033 rests on, that a sterile machine holds nothing anyone owns, would become a runtime condition instead of a statement about what the artifact is.

There is also a measured obstacle.
[The restore stage timeline](../evidence/2026-08-30-x86_64-restore-stage-timeline.md) records that a restored guest already observes a large time jump today, with `KVM_SET_CLOCK` applied and `IA32_TSC` among the restored MSRs, and reports guest uptime of about 190, 429, and 629 seconds across successive restores of one snapshot.
The netdev transmit watchdog fires on the jump, floods the console, and the shutdown acknowledgement then fails, so that run could not prove cleanup.
Suspension makes this strictly worse, because the gap between capture and resume stops being the age of a build artifact and becomes an arbitrary wall-clock interval chosen by whoever resumes.

## Decision

SOMA gains a second, differently named artifact, the Suspension, and a lifecycle pair that produces and consumes it.
A Suspension is a per-Instance tenant object.
It is never a Generation snapshot, it is never a Generation, and there is no path that turns one into the other.

### The artifact and its boundary

A Suspension is written with its own container magic, `SOMASUS`, at schema 1, and the Generation restore path rejects it on the magic alone, exactly as [ADR 0032](0032-candidate-bound-snapshot-identity.md) has snapshot schema 1 and Generation manifest schema 1 fail closed rather than being reinterpreted.
The Generation snapshot decoder never sees a Suspension and the Suspension decoder never sees a `SOMASNP` object, so a type confusion between a shared artifact and a tenant artifact is a decode failure rather than a review question.

A Suspension contains exactly four members:

1. the guest memory image as it stood at the suspension point,
2. the private writable overlay head as it stood at the same point,
3. a state manifest carrying the vCPU state, the device model, and the platform state, in the fixed order the snapshot codec already uses, and
4. a header binding the exact `GenerationId` the Instance was launched from, the machine shape, the owning tenant identity, the HostProfile that wrote it, the suspension point marker, the creation time, and the expiry.

It must never contain launch page material, a session key, a Noise transcript or handshake state, the per-Instance responder static secret, the readiness challenge, a TAP descriptor or lease generation, an activation authority, a network address or its lease, a host path, a host file descriptor, a capacity reservation, or a registry credential.
The vsock CID and the placeholder MAC that are unavoidably inside the captured device model are inert snapshot facts under the reasoning [ADR 0033](0033-sterile-restored-machine-authority-boundary.md) already gives, and resume treats them as such.

### The suspension point

Suspension has a certified capture point, in the same sense ADR 0030 gives the Generation pipeline one, and it is not the arbitrary instant a request arrives.

The Host Runtime sends an authenticated `Suspend` frame over the existing control session.
The guest agent completes or refuses any in-flight command, brings the virtio-net link down, flushes filesystems, retires the control session and zeroizes its Noise state and its per-Instance responder secret, forgets the readiness challenge, revokes and erases any secret whose declared delivery is re-deliverable, prints a suspension point marker on its console, and blocks in a disconnected suspend wait that is structurally the same wait ADR 0030 captures at.
The host then runs the existing quiesce order: ingress disabled, device work drained, overlay flushed, vCPU paused, queues proven quiescent, and only then reads memory, overlay, and state.

The reason for retiring the session inside the guest before the read, rather than scrubbing after it, is the one ADR 0030 states: the material was created here, so it cannot be uncreated, but a machine that has already dropped its authority cannot have that authority resurrected by anyone who later reads the bytes.
This is a statement about authority, not about confidentiality.
It does not make the memory image safe to disclose, and nothing below relies on the guest having erased anything.

### Privacy class, location, encryption, and destruction

A Suspension is tenant data at rest for its whole life, and it is handled as such regardless of what the guest did or did not erase.

It lives in a suspension store rooted separately from the prepared Generation store and separately from the writable head root, owned by the `soma-hostd` lifecycle identity, and reachable by no other identity on the Host.
It is addressed by an opaque random `SuspensionId` and not by a content digest.
Content addressing is right for Generations, which are shared on purpose, and wrong here: a store keyed by digest deduplicates identical tenant bytes across tenants and turns the question of whether an object already exists into a cross-tenant oracle.
A digest is still recorded inside the durable record for integrity, and a Suspension whose digest does not verify is destroyed rather than resumed.

Each Suspension is encrypted at rest under a per-Suspension data key, and that key lives only in the Host Runtime's durable record, sealed under a Host key that never enters the suspension store.
Only `soma-hostd` may read a Suspension.
The jailed `soma-vmm` process receives exact descriptors and never a path, which is [ADR 0031](0031-persistent-host-runtime-ownership.md)'s existing rule and needs no exception here.

Encryption is not decoration, because it is also how destruction is proven.
Destroying a Suspension unlinks its members and destroys its data key, and the durable cleanup ledger records both.
SOMA does not claim cryptographic erasure of the underlying media: on a reflink filesystem an unlink is a reference count decrement, and blocks may remain readable to anyone with raw device access until they are reused.
What SOMA claims is that the key is gone and the durable record says so, which is a claim it can actually keep.
Storing the object in the clear and relying on directory permissions was rejected for exactly this reason, since permissions are one mistake deep and give destruction nothing to mean.

A Suspension is destroyed on an explicit Destroy, on the terminal cleanup of the Instance that a resume produced, on expiry, on retirement or decertification of the Generation it names, and on any integrity failure.

### Lifetime and the sandbox that is never resumed

Every Suspension carries an expiry stamped at creation, and there is no indefinite retention.

E2B keeps paused sandboxes indefinitely and the survey records the conflict; the other providers differ, with Daytona documenting auto stop and auto archive.
SOMA takes the finite side, for a reason that is about ownership rather than about parsimony.
A Suspension is disk, and disk on a specific Host, because the header pins the HostProfile.
An object with no expiry is a capacity commitment with no end, made by a tenant, against a Host that cannot decline it later.
The retained development evidence is a reminder of how that ends in practice: SOMA's own live runs have filled a development disk with overlay copies.

The retention window is declared in the Template, bounded by a Host ceiling, and defaulted rather than unbounded.
Expiry is a durable operation with a receipt, not a garbage collection side effect, so a tenant whose Suspension is gone can be told when it went and why.
Each suspension starts its own clock, so a sandbox that is repeatedly resumed and re-suspended does not accumulate an ever older object.
Admission reserves suspension store bytes at the moment suspension is requested, and a Launch that declares suspension capability is admitted against that ceiling, so a Host cannot be talked into a commitment it has no room for.

### Resume restores memory and filesystem together

Resume restores both members or neither.

[The snapshot format](../research/snapshot-format-v2.md) already says why: the captured overlay is the disk the captured memory's page cache describes, and the two are consistent only as a pair.
A filesystem-only resume would place a memory image over a disk it does not describe, which is not a weaker mode, it is a corrupt one.
E2B's filesystem-only mode is a different object with a different capture procedure, an export taken after a clean unmount rather than a suspension, and if SOMA ever wants one it will be a separate artifact with its own decision, not a flag on this one.

Resume is a distinct constructor and does not reuse `restore_sterile`.
A sterile machine under ADR 0033 exists before anyone owns it and is safe to hold in a pool; a resumed machine is owned from the first instruction, because the tenant's bytes are already in it.
`resume_suspension` therefore requires the owning tenant identity and the `SuspensionId` before it constructs anything, and its product can never enter the prepared worker pool.
It then performs the same shaped mutations `assign` performs, validating and installing a fresh vsock CID and a fresh private head before the vCPU may run, and it mints entirely fresh Instance authority: a new Instance identity, a new launch page, a new responder secret, a new readiness challenge, and a new authenticated session.
The Suspension itself is immutable and is cloned rather than opened for write, so a failed resume cannot damage the saved state and two resumes cannot share a head.

### What the guest observes about time

The guest is not lied to about wall-clock time.
On resume the wall clock is set forward to true current time, because a sandbox that comes back believing it is still yesterday will write wrong timestamps, fail certificate validation, and mislead every log it produces.

That decision makes the jump the guest observes larger, not smaller, so the design has to carry it rather than hide it.
Three parts do.

First, the link is down across the boundary.
The guest agent brings virtio-net down before the suspension point and up after resume, so the transmit watchdog has no in-flight queue to time out on across the gap.
This directly targets the failure the retained timeline recorded, where the watchdog flood is what made the shutdown acknowledgement fail and cleanup unprovable.

Second, the gap is told rather than inferred.
Resume delivers a suspension notice through the same one-use launch page path that already carries fresh launch material, naming the capture time and the resume time, and the agent exposes it so guest software can react deliberately instead of deducing that the machine has travelled through time.

Third, the agent restarts the time-sensitive services it owns after resume, in a fixed order, before it reports Ready, so readiness continues to mean what it means today.

The existing defect is a blocker rather than a footnote.
The timeline at `c0fd993` shows the jump on a build artifact that is hours old at most, so the mechanism is already wrong before suspension exists.
The growth of reported uptime across successive restores of a single snapshot, about 190 then 429 then 629 seconds, tracks the age of the snapshot rather than the duration of the restore, which is the signature of the guest reading elapsed time from a host-referenced paravirtualized clock rather than from the restored TSC.
That is a hypothesis, stated as one.
Suspension must not be implemented until it is settled and the watchdog failure is gone, because building a feature whose whole purpose is a long gap on top of a clock path that already mishandles a short one would produce a capability that fails on its first real use.

### Network, CID, overlay head, and secrets across the boundary

The network bundle is released at suspension and is not held.
A suspended sandbox owns no namespace, TAP, veth pair, conntrack zone, address lease, nftables handle, DNS reservation, ingress reservation, or activation authority, and resume claims a fresh bundle and activates it only after Ready, which is the order [ADR 0012](0012-fail-closed-networking.md) and the Launch transaction already fix.
Holding the lease across the gap was rejected because it makes an idle object hold scarce Host network state for the whole retention window and because a retained activation authority is exactly the reusable authority this repository keeps refusing to create.
The cost is real and must be said plainly: the sandbox's address is not stable across a suspension, every long-lived TCP connection is dead on resume, and any future preview URL changes.

The vsock CID is a Host allocation and is released with everything else.
The captured CID inside the device model is an inert fact, and resume validates and installs a fresh one before the machine can run, under ADR 0033's reasoning.

The private overlay head stops being a lease and becomes the Suspension's disk member.
It is flushed, sealed into the store, and removed from the writable head root, so no head lease survives its Instance and no two owners can reach one head.
Resume clones from that member into a fresh private head and leaves the member untouched.
On the provisioned XFS profile the clone is a reflink and costs what is written; on a store without reflink it is a full copy, and the retained development evidence measures that fallback at seconds of wall clock and about ten gibibytes per launch, which is the honest upper bound for a resume on an unprovisioned Host.

Delivered secrets are the hardest member and get the strictest rule.
A secret that reached the guest is in guest memory and possibly on the guest disk, and it would therefore be in the Suspension for the whole retention window, long after the Instance that was trusted with it is gone, and without the issuing system having any idea.
So the Template's secret declaration gains a suspension disposition.
A Generation with any declared secret may not be suspended unless every such secret declares itself re-deliverable, in which case the agent revokes and erases it before the suspension point and the Host re-resolves and re-delivers a current value on resume through the one-use launch page path.
This is deliberately not a claim that the Suspension contains no secret bytes, because best-effort erasure inside an untrusted guest cannot support that claim; a copy may survive in a child process, a log file, or the page cache.
The claim is narrower and defensible: SOMA does not itself preserve a credential it delivered, resume gets the current value rather than a stale one, and whatever residue remains is covered by encryption at rest and by key destruction.

### Idle timeout, and what activity means

Idle is evaluated by the Host Runtime, which is the only component with a complete view of an Instance, and the resulting suspension or destruction is a durable operation with a receipt, indistinguishable in evidence from one a caller requested.

Activity is host-observable and never guest-asserted.
It is an accepted lifecycle operation against the Instance, an attached client session with a frame inside the window once the guest protocol carries one, or, when networking is wired, an admitted ingress packet.
Guest CPU consumption is not activity, because a busy loop would keep an abandoned sandbox alive forever and because the guest is untrusted and would then be setting its own bill.
Guest-originated egress is not activity by default either, since a background poller has the same effect as a busy loop; a Template may opt into counting it, and the Host ceiling still applies.
Modal counts live TCP and this is a deliberate divergence from that.

Two independent bounds exist.
The idle timeout suspends the Instance if its Generation permits suspension and destroys it if not.
A maximum lifetime bounds the whole Instance regardless of activity, so no amount of traffic makes a sandbox permanent.
Both are declared in the Template and bounded by Host ceilings, and the Host ceiling wins.

### A Suspension never becomes a Generation

There is no promotion path from a Suspension to a shared image, and this closes a capability three surveyed providers advertise, since snapshotting a running sandbox to skip dependency installation is how E2B, Vercel, and Modal expect templates to be built.

SOMA's answer to that use case stays the Generation compiler: a tenant who wants a reusable image writes a Template and builds a Generation, whose snapshot is captured before any Instance exists and is therefore shareable by construction.
Admitting a tenant machine's memory into the Generation store would carry tenant state into every future Instance of that Generation and would destroy the one property ADR 0030 and ADR 0033 are both built on.
Offering it later would require a certified second capture point, an agent transition that provably retires everything a tenant session created, and evidence that the retirement leaves nothing behind, which is the same bar ADR 0030 already set for warm-cache capture and did not clear.

## Consequences

SOMA gains pause, resume, reconnectable state, and an idle policy, and it gains them without weakening the sterile pool, because the tenant object and the shared object are different types that cannot be decoded by each other's readers.

The cost lands in several places and none of it is free.

Suspension is not fast.
Writing a memory image and sealing an overlay head is proportional to memory size and to what the tenant wrote, and E2B's published figure of roughly four seconds per gibibyte to pause is the right order of magnitude to expect rather than a target to beat.
Encryption adds a pass over every byte in both directions, so resume is slower than the 2.71 ms machine construction the retained timeline records for the prepared path, and a Suspension resume can never be a prepared worker.

Suspension pins capacity.
The header names the HostProfile, so a suspended sandbox is resumable on one Host, and Host maintenance either waits for expiry or destroys tenant state.
Cross-Host resume is out of scope here because the captured device model, CPUID template, and kernel are Host and Generation specific, and pretending otherwise would produce a resume that boots into an undefined machine.

Suspension adds a durable tenant data store to a system that previously had none, which brings key management, retention policy, capacity admission against disk, expiry receipts, and destruction evidence, all of which are now part of the Host Runtime's job rather than optional extras.

Resume breaks continuity the tenant may not expect.
The address changes, connections are dead, delivered secrets are re-resolved rather than preserved, and the wall clock jumps forward by the length of the gap.
Each of those is a deliberate choice above and each will surface as a support question.

Networking, the guest protocol, and the persistent Host Runtime remain upstream of all of this in [the gap map](../research/gap-map.md)'s order, and nothing here reorders them.
This decision is Design-class evidence only, under the [engineering standard](../standards/sota-engineering-standard.md), and no capability described here may be stated as anything above Designed until it runs.

## Alternatives considered

Adding a tenant flag to the existing snapshot format was rejected because it puts tenant bytes within reach of the restore path the prepared pool uses, and turns a property of the artifact into a condition checked at runtime.

Keeping paused sandboxes indefinitely, as E2B does, was rejected because a Suspension is an unbounded capacity commitment made by a tenant against a Host that pinned itself to it.

Restoring only the filesystem was rejected because the captured overlay and the captured memory are consistent only as a pair.

Freezing the guest clock so that no time appears to pass was rejected because it makes every timestamp the guest writes wrong and every certificate check unreliable, in exchange for hiding a jump that the guest's timers will observe anyway.

Holding the network bundle across suspension was rejected because it retains scarce Host state and reusable activation authority for the whole retention window.

Preserving delivered secrets inside the Suspension was rejected because it stores a live credential past the life of the Instance that was trusted with it, with no signal to whatever issued it.

## Open, with the measurement that would settle each

**Whether setting the wall clock forward on resume is sufficient, or whether the guest needs a real suspend and resume path.**
The alternative is to drive the guest through an actual power management transition before capture and after restore, so that the kernel itself re-arms timers, restarts queues, and re-reads the clock the way it does after a laptop lid closes, instead of being restarted mid-flight with a corrected clock.
That is a much larger change to the guest contract and it should not be adopted on a guess.
The measurement that settles it is a graded gap run on a real Linux KVM Host: capture one Instance at the suspension point, resume it after gaps of about one second, one minute, one hour, and twenty-four hours, and record for each gap which guest subsystems misbehave, naming the netdev watchdog, the timer wheel, kernel work queue stalls, systemd timers, and any TLS validation failure, with the guest console and the machine timeline retained per gap.
If failures appear only above some gap length, the clock correction is enough with a bounded gap; if they appear at every gap length, the power management path is required.

**Whether the current time jump is a paravirtualized clock reference rather than the TSC.**
The stated hypothesis is that the guest reads elapsed time from a host-referenced clock and therefore sees the artifact's age.
The diagnostic that settles it is to record, on one Host, the guest's `CLOCK_MONOTONIC`, the guest's `CLOCK_REALTIME`, the raw TSC, and the paravirtualized clock's system time field immediately after resume, for three restores of one snapshot taken at known and different artifact ages, and to check which of them scales with the artifact's age rather than with the restore duration.
This is a cheap run and it is a precondition for the graded gap run above.

**What suspension actually costs per gibibyte on a provisioned Host.**
The four seconds per gibibyte in the survey is a vendor claim about a different system and must not be used as a SOMA figure.
The measurement is a capture and resume ladder on the certified XFS reflink profile, with memory write, overlay seal, encryption, decryption, and head clone timed separately, so that the encryption decision above can be re-examined against a number rather than an assumption.

**Whether the idle window's default and the retention default are right.**
Both are policy and neither can be chosen from first principles.
What would settle them is Host-side distributions once the Host Runtime owns Instances: the observed distribution of gaps between operations on real sandboxes, and the observed distribution of intervals between a suspension and its resume, with the defaults set below the tail that would strand real users rather than at a round number.

## Verification gates

- The Generation restore path must reject a `SOMASUS` object and the Suspension path must reject a `SOMASNP` object, on the container magic, before any member is read.
- A Suspension produced by the certified suspension point must be scanned and must contain no decodable launch page, session key, responder secret, readiness challenge, network lease, activation authority, or Host path.
- `resume_suspension` must be unreachable without an owning tenant identity, and its product must be structurally incapable of entering the prepared worker pool.
- Resume must install a fresh CID, a fresh private head, fresh Instance identity, and fresh launch authority before the vCPU may run, and any failure must destroy the machine rather than return it anywhere.
- Two concurrent resumes of one Suspension must produce two distinct private heads and must leave the Suspension unmodified, proven by digest before and after.
- Destroying a Suspension must destroy its data key and record both the unlink and the key destruction in the cleanup ledger, and a subsequent read attempt must fail closed.
- Expiry must run as a durable operation with a receipt, and an expired Suspension must be unresumable and gone from the store.
- Admission must refuse a suspension request that exceeds the Host's suspension store ceiling, without partially writing an object.
- A Linux KVM run must show a resume across a gap of at least one hour that reaches authenticated Ready, executes a command, and proves complete cleanup, with no netdev watchdog line on the guest console.
- A Generation declaring a secret that is not re-deliverable must be refused suspension at request time rather than at capture time.
- The idle evaluator must not treat guest CPU consumption as activity, proven by a guest busy loop that is suspended or destroyed on schedule.
