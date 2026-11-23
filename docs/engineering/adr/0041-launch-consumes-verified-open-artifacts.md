# ADR 0041: Launch consumes verified open artifacts

## Status

Accepted.

## Context

Generation admission verifies content-addressed artifacts before allowing Launch.
The KVM backend previously discarded that verification handle and reopened artifact paths later during boot, pool replenishment, and jailed-worker construction.
Snapshot restoration also read a sibling `snapshot/` directory even though the certified Generation already names the memory, state, and overlay objects by digest.
A privileged host process could replace a path between verification and use, so certification did not bind the bytes ultimately consumed by KVM.

## Decision

Every launch artifact is opened through its certified descriptor with digest and size verification.
The handle returned to the launcher is the same handle whose bytes passed verification.
Snapshot memory, state, and overlay objects come from the descriptors embedded in `SnapshotBinding::Captured`, never from a sibling directory convention.
Boot, prepared-pool replenishment, private-head cloning, and jailed-worker construction receive owned or duplicated handles rather than resolving artifact paths.

Paths remain operator-facing discovery inputs for locating a prepared Generation.
They stop being launch capabilities once admission has resolved and verified the Generation.

## Consequences

A rename, replacement, symlink change, or deletion after admission cannot change the bytes an admitted launch consumes.
Deleting an object before it is opened causes a typed refusal instead of falling back to an unverified copy.
Each virtual machine owns its duplicated handles and closes them during teardown.
The pool key uses certified object identities rather than mutable host path text.

This change does not make an uncertified Candidate launchable and does not weaken compatibility checks.
