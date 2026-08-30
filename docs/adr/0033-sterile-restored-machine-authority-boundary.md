# ADR 0033: A restored prepared machine receives authority only after assignment

- Status: Accepted
- Date: 2026-08-30
- Extends: ADR 0006, ADR 0024, ADR 0030, and ADR 0031

## Context

Restoring KVM memory, platform state, devices, a vCPU, and event resources on every Launch consumes most of the warm request path.
A prepared worker can pay those invariant costs before an Instance exists, but it must not retain reusable tenant authority.
Captured device state necessarily contains the non-secret placeholder MAC and captured vsock CID that the guest observed before capture.
Those inert configuration bytes are not an assigned identity because the restored vCPU cannot start and the machine is not publicly accessible before assignment.

## Decision

`restore_sterile` may construct a stopped machine from admitted immutable artifacts and restore the captured device model into it.
The resulting `Sterile` type exposes only one consuming transition named `assign`.
It contains no Instance identifier, launch page, private overlay head, TAP lease, command, credential, tenant byte, readiness challenge, or live guest session.
The captured MAC and CID remain inert snapshot facts until assignment because no method can start the vCPU, publish a launch page, or reach the device bus from `Sterile`.

Assignment validates the fresh CID and private-head shape before committing device mutations.
Any failure consumes and destroys the machine rather than returning it to the pool.
Only after device assignment succeeds does the restore sample its single-use readiness challenge.
The assigned machine still cannot report Ready until fresh launch material is consumed and an authenticated repaired guest session completes the readiness proof.

The placeholder MAC is not production network identity.
A future live network assignment must replace it with the admitted per-Instance MAC, IP, lease generation, TAP descriptor, and activation authority before Ready.

## Consequences

Prepared restoration can move KVM construction off the Launch path without moving authentication authority into a reusable pool.
The inactive captured CID is a machine-format placeholder rather than a host allocation.
The distinction must remain enforced by type privacy, a stopped vCPU, consuming assignment, and end-to-end Linux KVM tests.
This decision does not wire the pool into `soma-hostd`, create a production `soma-vmm` process, or prove a latency target.

## Verification gates

- Portable architecture and compile checks must prove the sterile API cannot start or expose the machine before assignment.
- Linux KVM tests must prove wrong head shape and invalid CID destroy the worker without reaching Ready.
- Linux KVM tests must prove one successful assignment, authenticated readiness, command execution, and complete cleanup.
- Concurrency tests must prove distinct Instance, CID, network, session, disk, and writable memory identity.
- The production Launch path must use the `soma-hostd` single-winner claim before reporting `PreparationClass::Prepared`.
