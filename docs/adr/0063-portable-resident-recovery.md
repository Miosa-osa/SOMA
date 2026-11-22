# 0063. Portable resident recovery

Status: Proposed

## Context

Resident checkpoints can contain a mounted workspace in addition to guest RAM, machine state, and the writable system disk.
The current host can restore a workspace checkpoint into a new sandbox when the original owner records still exist on that host.
It refuses a mounted checkpoint on the same sandbox ID, and a separate host cannot import the checkpoint without copying the source's live registry records.

## Decision

Keep recovery authorization in the broker and workspace ownership in the host volume store.
A point-in-time recovery can replace a workspace backing only if the destination disk is detached and the broker has reserved that sandbox and disk for this restore.
Keep the current backing available as the rollback version until the recovered worker is ready and the broker records successful recovery.
An ordinary restart of a stopped sandbox continues to boot its current disk.
A linked restore operation is the only path that publishes a replacement backing.

The source broker exports one immutable checkpoint and supplies its manifest and receipt to storage.
The receipt binds the source sandbox, tenant, checkpoint operation, Generation, machine shape, and geometry and digest of every artifact.
Managed storage seals all artifacts with authenticated encryption and publishes the checkpoint manifest after verifying each uploaded object.
Bulk transfer uses bounded streaming and does not expose private host paths through the API.

A destination import requires authorization for the exact tenant, source checkpoint, target sandbox, and restore operation.
It checks the authenticated manifest, installed Generation, geometry, compatible host profile, resource reservation, and each artifact before making the import visible.
The destination creates host-local ownership records from that authorization.
A resident restore refused for a host profile mismatch reports a distinct incompatibility for that placement, and the checkpoint remains available for a compatible host.
It reconstructs the virtual machine instance with fresh local host capabilities and guest authentication.
Source registry files, leases, network state, encryption key records, and arbitrary archive member paths are outside the import contract.

The host records the restore intent durably before handing off the staged disk and leaves the old workspace untouched while the restored worker is starting.
An uncommitted restore can be cleaned up without writing the old backing.
After readiness, publication uses one durable volume-store transition with an expected prior backing and a fixed operation ID.
An exact retry observes the staged or committed operation, and a different request cannot reuse its disk or operation.
If readiness was published but the final response was lost, a replay returns the committed result.
An interrupted, uncommitted restart recovers the prior backing before allowing a new operation.
Rollback is separately authorized and requires the recovered worker to have stopped and released the replacement disk.

## Acceptance

Prove that a checkpoint created with a mounted disk retains the original process and its saved files after the source shuts down.
Write a distinct file marker after capture and verify the former disk remains available on failure and through an authorized rollback.
Reject modified artifacts, mismatched shape or Generation, a different tenant, and a replay with a changed destination.
Recover from termination before staging, during transfer, before publication, and immediately after publication.
Run the managed S3 backup and restore on two physical hosts using the intended release binary and image.
