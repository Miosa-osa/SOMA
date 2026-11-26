# 0068 - Attested checkpoint memory

Status: Accepted for implementation

## Context

ADR 0058 has every resident restore and fork hash the whole checkpoint memory file before it creates the VM.
That read is seconds for a desktop with gigabytes of RAM, and it repeats on every restore and every fork of the same unchanged file.
The Generation store already avoids the same cost with its admission ledger: a blob hashed once is recorded by its kernel stamp, and a later admission of the same stamp skips the read.
A stamp is the device, inode, size, and modification and change times, with nanoseconds.
The change time moves on any write, resize, rename-over, or permission change and cannot be set from user space.

## Decision

The broker that owns the private checkpoint directory hashes the published memory file once and records an attestation beside it in `memory.attest`.
An attestation is the memory digest and the stamp the file had while it was hashed.
The broker records it after the directory rename that publishes the checkpoint, so the stamp is the one a restore sees.
Recording follows a fixed order.
The stamp is taken before hashing and must be unchanged after it.
Hashing starts only when the clock is at least 50 milliseconds past the stamped change time, so a later write cannot share that change time on a coarse clock.
The file is synced after the stamp, which cleans and write-protects any shared mapping, so a later write through a mapping a writer kept moves the change time as a write call does.
A failure at any step leaves no record and does not fail the capture.

A restore or fork reads the record and hands it to the sealed worker in a `memory_attestation` request that precedes `recover` or `fork`.
The worker checks the descriptor it is about to map.
It skips the hash only when the descriptor's stamp equals the attested stamp, the descriptor's size equals the attested size, and the attested digest equals the manifest's memory digest.
Any other case hashes the file exactly as before, so an absent, damaged, or stale record costs a hash and is never a refusal or a skip.
The worker still checks size and digest against the manifest, so an attestation can only replace a read and cannot admit bytes the manifest does not name.

A worker that predates the request rejects that one packet, and the broker continues with the recovery, which then hashes the memory.
An imported bundle may not carry a record, because only the broker that hashed a file can vouch for its stamp on this host.
The stamp type is shared with the Generation admission ledger, and the ledger's file format is unchanged.

## Trust

The record is trusted exactly as far as the checkpoint directory is.
Only the broker can write that directory; the worker receives descriptors and never a path.
Whoever can write the record can already write the memory file, so the record grants no new authority.
A writer that keeps a descriptor to the memory file after capture moves the change time with any later write, so its file is hashed.
This relies on the checkpoint filesystem updating file times on shared-mapping writes, which XFS and ext4 do through their page-write fault handlers and tmpfs does not.
A root process that steps the wall clock backwards can make a later write reuse an earlier change time; root is already inside the broker's trust boundary.

## Consequences

A capture pays one extra read of the memory file after publication, and every restore and fork of the unchanged file skips it.
The root, saved overlay, private head, and durable volume are still hashed on every restore.
A checkpoint imported from another host has no record and is hashed on every restore until a later change records one.
