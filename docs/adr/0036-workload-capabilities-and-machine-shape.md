# ADR 0036: One shape per Generation, and the capability boundary around what a sandbox may contain

- Status: Accepted
- Date: 2026-08-31
- Extends: ADR 0009, ADR 0012, ADR 0030, and ADR 0033
- Relates to: ADR 0034

## Context

The [capability survey](../research/sandbox-provider-capability-survey.md) named four dimensions on which SOMA has an implementation but no decision, and the [gap map](../research/gap-map.md) ranked one of them, shape, as structural because it is a decision rather than work.
The four belong in one record because each of them asks the same question in a different place: what may a sandbox contain, and how large may it be.
Answering them separately would let three answers quietly assume a fourth that was never settled.

Shape is the one that is already fixed by code rather than by choice.
`MachineShape` carries a vCPU count, a memory size in MiB, and a storage size in MiB, and the command line accepts all three, but only one of the three reaches the machine as a variable.
The vCPU count is fixed at one by the `x86_64` machine contract, because the cold-boot path and the restore path both call `create_vcpu(0)` and create nothing else.
The memory size must exactly equal the size the Generation's snapshot was captured with, because `compatibility::check_header` compares the requested size against the manifest and returns `Incompatibility::MemoryLayout` when they differ.
The overlay size comes from the Generation's template rather than from the request.
So each Generation today has exactly one launchable shape, and that is a stronger constraint than a default, because a default can be raised and a captured memory image cannot.

The other three dimensions have no implementation at all, and each of them is blocked by something specific rather than by effort.
A container runtime inside the guest needs egress, which the KVM Backend does not wire, and needs a writable layer its own storage driver can use, which the guest root does not offer because it is EROFS with an ext4 private overlay through OverlayFS.
A GPU needs PCI, VFIO, and an IOMMU, and the version 1 machine contract admits none of them.
Durable storage needs an owner that outlives the Instance, and ADR 0031 has decided that owner and has not built it.

The [engineering standard](../standards/sota-engineering-standard.md) requires every optional capability to activate its accompanying safety work before it is admitted, and requires a deferral to be an explicit state rather than an absence.
This record therefore decides two of the four outright, and draws a permanent boundary around the other two while deferring the capability itself against a named condition.

## Decision

### Shape is a Generation build parameter, and a request asserts it rather than choosing it

SOMA offers no per-Instance range of CPU, memory, or disk.
A Template declares one shape, the Generation compiler builds and certifies one machine at that shape, and every Instance of that Generation launches at exactly that shape.
Offering a caller two memory sizes means building two Generations from the same Template Lock, each with its own boot, its own capture, its own certified snapshot, its own `GenerationId`, and its own prepared worker pool.

The `MachineShape` in a Launch request keeps its three fields and changes its meaning.
It is an assertion about the Generation the caller believes it is launching, checked before any resource is acquired, and not an instruction to the machine.
A request whose memory does not equal the captured size is already rejected as a memory-layout incompatibility, and that rejection becomes the general rule rather than an artifact of the restore path: a shape field that disagrees with the admitted Generation fails the launch with a typed incompatibility naming the field.

A request for more than one vCPU is rejected rather than satisfied with one.
The receipt's honest reporting of the contract's single vCPU as an observed value is correct evidence and is not a substitute for admission, because a caller that asked for four and received one has been given a weaker machine than it requested without being told, which is exactly the silent fallback the standard forbids.
Raising the vCPU count is a change to the machine contract and to the one-thread-per-vCPU property, not a change to the request schema, and it is out of scope here.

### A nested container runtime never runs its storage driver on the private overlay

If SOMA ever supports a container runtime inside the guest, that runtime receives its own writable block device with its own filesystem, and never a directory on the OverlayFS mount that composes the EROFS root with the private ext4 overlay.
Stacking a container storage driver on OverlayFS is a known failure surface, and the guest's root composition is not negotiable because it is what makes the root immutable and shared and the writable state private.
This part is decided now, in advance of the capability, because it is the part that would otherwise be settled accidentally by whoever first tries to make a container run inside a sandbox.

The capability itself is deferred.
It is settled when three things are true together: egress is Integrated rather than designed, so that an image pull can happen at all; the minimal device surface admits a second writable block device, which is a change to the version 1 machine profile and to the five virtio-mmio devices it fixes; and a retained live run on the certified host profile proves that a named runtime creates and runs a container on that separate device, with the guest privilege it needs stated exactly rather than granted broadly.
Until all three hold, SOMA states that nested container runtimes are unsupported, which is a weaker product claim than Vercel's and a truthful one.

### The GPU is a different machine profile, not a shape field

SOMA does not attach a GPU, and adding a field for one to `MachineShape` would be dishonest, because the obstacle is not the request schema.
Passing a physical device through requires PCI, VFIO, and an IOMMU, and the version 1 machine contract admits no PCI at all.
The deeper obstacle is that a passed-through device cannot be captured into a snapshot and restored into a sterile prepared worker, so a GPU sandbox would launch by cold boot and would abandon the restore path that the whole prepared worker design and the measured warm numbers rest on.

A GPU would therefore arrive as a separately certified machine profile with its own device contract, its own compatibility rules, its own performance boundary declared for a cold path, and its own conformance matrix.
The condition that would open it is a workload that justifies a second machine profile, together with a certified host profile that provides IOMMU-backed device isolation strong enough for a hostile guest.
Neither exists, and the honest position is that a second machine profile is a larger commitment than the feature it would deliver.

### Durable storage attaches as a separate volume or not at all

The private overlay keeps the semantics ADR 0002 and the storage standard give it.
It is created per Instance, unlinked at launch, never shared, and destroyed with the Instance, and no durable storage feature may change that by making the overlay survivable, because the overlay is the object that carries the guarantee that mutable tenant state is not reused.

Durable storage, if SOMA offers it, is a distinct named volume with its own device, its own explicit attach and detach transactions, its own quota, its own crash recovery, and its own deletion semantics, owned by the Host Runtime rather than by the Instance.
It is excluded from every snapshot without exception.
That exclusion is the part this record owes ADR 0034, which decides the privacy class of a per-sandbox snapshot: whatever that record concludes about tenant state inside a captured image, an attached durable volume is not in the image, so its contents are governed by the volume's own deletion semantics and not by the snapshot's.
This record does not decide anything about the per-sandbox snapshot itself.

The capability is deferred, and it is settled by ADR 0031 being implemented rather than accepted, because a volume that outlives its Instance needs an owner that outlives the launching process, and by the accompanying primitives the standard requires for persistent storage being designed as one transaction rather than added one at a time.

Mounting remote object storage through FUSE inside the guest is decided against as a platform feature.
It reduces to egress plus credential delivery plus a workload the user installs, and every one of those three is already a named gap with a named owner.
Adding a platform surface for it would create a fourth thing to certify that delivers nothing the first three do not.

## Consequences

The shape decision is the expensive one, and its cost lands on Generation construction, storage, and capacity planning together.

Every additional shape is a full Generation build: an image import, a rootfs normalization, a boot, a quiesce, a capture, and a certification, so build time multiplies by the number of shapes rather than amortizing across them.
Storage multiplies in the dimension that matters most, because `memory.raw` is the whole memory size on disk: a Template offered at 512 MiB and at 8192 MiB costs 8.5 GiB of captured memory per revision, before overlay and state, and every Template revision pays it again.
Capacity planning is where it hurts longest.
The prepared worker pool is keyed by exact HostProfile, GenerationId, CPU and memory class, overlay class, and network profile, so shapes multiply pools rather than dividing one, and each pool carries its own minimum, target, and maximum.
A host that holds one warm worker per pool holds one per shape per Generation, and the memory reserved for warm workers rises with the sum of the shapes rather than with the largest of them.
This is why offering a range is a capacity decision before it is a product decision, and why the answer here is that operators choose shapes deliberately, one Generation at a time, rather than callers choosing them per request.

The honest competitive consequence is that SOMA's resource dimensions are narrower than E2B's documented 1 to 8 vCPUs and 512 to 8192 MiB, and will stay narrower until the machine contract changes.
A caller who needs four cores cannot use SOMA, and the correct response to that caller is a rejection naming the machine contract rather than a launch that silently gives them one.

The container runtime deferral is the one that costs product reach.
Real build and test work is the category's main use, and a sandbox that cannot run a container runtime is excluded from part of it.
The deferral is nonetheless correct, because the three conditions are all real work with owners, and a runtime made to appear to work on top of OverlayFS would be a correctness problem discovered by a user rather than by a test.

The GPU decision costs nothing today and forecloses nothing, because it names the profile boundary rather than the feature.
The risk it carries is that the restore path and the prepared worker pool are now load-bearing for the performance story, so any future device that cannot be captured is a larger architectural decision than it will look like when someone proposes it.

The storage decision buys a boundary at the price of a slower feature.
Drawing the line before the capability means the overlay's guarantee is safe from a later feature that would have been easier to build by weakening it, and it means ADR 0034 can reason about snapshot contents without also reasoning about attached disks.

## Verification gates

- Contract tests must prove that a shape field disagreeing with the admitted Generation is rejected with a typed incompatibility naming the field, for memory, for storage, and for vCPU independently.
- Contract tests must prove that a request for more than one vCPU is rejected at admission rather than reported as an observed one.
- Prepared worker pool tests must prove that two Generations built from one Template Lock at different memory sizes occupy separate pools and that a worker from one is never claimed for the other.
- Compatibility tests must prove that the captured memory size, the overlay size, and the vCPU count are all read from the admitted Generation and never from the request.
- Any future nested container runtime work must present a retained live run on the certified host profile showing the runtime operating on a separate writable device, and must not pass a gate that runs it on the OverlayFS mount.
- Any future durable storage work must prove attach, detach, quota enforcement, crash recovery, and deletion as one transaction, and must prove that no snapshot capture includes a byte of an attached volume.
