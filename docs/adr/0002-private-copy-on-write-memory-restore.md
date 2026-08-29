# ADR 0002: Restore memory from immutable private mappings

- Status: Accepted
- Date: 2026-08-28
- Decision owners: SOMA maintainers

## Context

A warm Generation contains a memory snapshot that may be restored concurrently into many Machines.
Eagerly copying the entire memory artifact for every Launch makes latency and memory bandwidth scale with configured guest RAM rather than the guest working set.
Mapping one writable memory object into multiple Machines would make cross-Instance mutation possible and violate the isolation contract.

SOMA needs snapshot fan-out that preserves independent mutable guest memory, minimizes work before vCPU resume, and fails closed when artifact compatibility or integrity is uncertain.

## Decision

The certified memory Artifact is immutable and content-addressed.
Each `soma-vmm` maps that Artifact as a private copy-on-write guest-memory region using Linux private mapping semantics.
The initial Linux policy is equivalent to `MAP_PRIVATE | MAP_NORESERVE` where supported and safe for the selected backing filesystem.
The launch path must not perform an eager full-memory copy or request eager population of every page.

The same immutable backing inode may supply clean pages to many Machines through the host page cache.
The first write to a page creates private storage owned only by the writing VMM process.
No writable memory file, anonymous page, userfault state, or delta head may be shared across Instances.

Full content hashing and provenance verification occur when a Generation is built, installed, or audited.
Launch performs a bounded fail-closed check that binds open immutable file handles to the certified manifest and expected filesystem identity.
Launch must avoid path re-resolution after verification and must reject mutable, replaced, truncated, sparse-layout-incompatible, or otherwise uncertified backing.

The Generation compatibility fingerprint binds at least the following values:

- SOMA snapshot format and runtime compatibility version.
- Host architecture and required CPU feature class.
- Guest physical memory layout and configured RAM size.
- Guest kernel and command line identity.
- KVM, vCPU, interrupt-controller, clock, and device state formats.
- Virtio device topology and negotiated feature sets.
- Memory Artifact identity, length, page geometry, and integrity evidence.
- Root filesystem base identity and private disk-head format.
- Guest-agent protocol and Repair contract version.

Any mismatch returns an incompatibility fault before vCPU resume.
SOMA does not silently cold boot, rebuild, translate, or select a weaker restore path.

## Alternatives considered

### Eager per-Launch memory copy

An eager copy gives every Instance independent bytes before KVM starts.
It is rejected because launch work and memory traffic scale with total RAM and destroy the intended warm-restore latency profile.

### Shared writable mapping

A shared writable mapping has low setup cost.
It is rejected because one guest could modify state observed by another guest or corrupt the reusable snapshot.

### Eager population of a private mapping

Eager population preserves private copy-on-write behavior while touching all pages before resume.
It is rejected as the default because it converts a working-set optimization into a full-memory launch cost.
Measured prefaulting of a bounded working set may be an operator policy, but it is not part of snapshot compatibility.

### Userfault-driven restore

Demand paging through `userfaultfd` can support remote or compressed artifacts and advanced working-set control.
It adds a page-fault server, more kernel interface surface, failure states during guest execution, and extra scheduling work.
It remains a later experiment that requires separate threat analysis and measurement.

### Process fork from a live template

Forking a live VMM can make process and memory cloning inexpensive.
It is rejected for the initial architecture because multithreaded process state, inherited descriptors, entropy, control channels, and writable device state create a larger correctness and isolation problem.

## Consequences

Warm Launch cost follows mapping, KVM restoration, and touched working-set pages rather than configured RAM size.
Host page-cache residency becomes an important experimental variable and must be reported honestly.
Memory pressure can produce major faults after resume, so benchmark metadata must distinguish cold and warm host cache states.
The immutable Artifact lifecycle must prevent replacement while any VMM retains a mapping.
Memory accounting and overcommit policy remain operator responsibilities and must not weaken per-Instance isolation.

## Verification

Linux KVM tests must prove that two Machines restoring the same Artifact see identical initial bytes and that a write by one Machine is invisible to the other.
Tests must reject altered metadata, truncated files, substituted inodes, incompatible CPU features, device-state mismatches, and writable shared backing.
Resident-set and major-fault measurements must demonstrate that Launch does not eagerly populate the entire configured guest memory.
Burst tests must verify independent mutable state under concurrent write pressure.
