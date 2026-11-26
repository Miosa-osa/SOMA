# 0068 - Attested immutable restore inputs

Status: Accepted for implementation

## Context

ADR 0058 has every resident restore and fork hash the checkpoint memory, the Generation root, the private writable head, and any durable volume before it creates the VM.
Those reads cost seconds per GiB and repeat on every restore and every fork of the same unchanged files.
The Generation store already avoids the same cost with its admission ledger: a blob hashed once is recorded by its kernel stamp, and a later admission of the same stamp skips the read.
A stamp is the device, inode, size, and modification and change times, with nanoseconds.
The change time moves on any write, resize, rename-over, or permission change and cannot be set from user space.

## Decision

A trusted broker hands the sealed worker an attestation for each restore input that nothing may write after publication: the checkpoint memory and the Generation root.
An attestation is a digest and the stamp the file had when that digest was known to describe it.
The worker skips a hash only when the descriptor it is about to map has that exact stamp and size and the attested digest equals the digest its record names.
Any other case hashes the file exactly as before, so an absent, damaged, or stale attestation costs a hash and is never a refusal or a skip.
The worker still compares the result with its record, so an attestation can only replace a read and cannot admit bytes the record does not name.

The private writable head and the durable volume are never attested and are always hashed.
They are writable by a running guest, so a stamp proves nothing about their content.

### Checkpoint memory

Capture already writes the memory, syncs it, and reads it back against the manifest's memory digest before it reports success.
After the directory rename that publishes the checkpoint, the broker records that digest with the file's stamp in `memory.attest`, without reading the memory again.
A directory rename leaves the stamps of the files inside it unchanged on XFS and ext4.
The record follows a fixed order.
The file is synced first, which cleans and write-protects any shared mapping, so a later write through a mapping a writer kept moves the change time as a write call does.
The stamp is taken, the record is made only once the clock is at least 50 milliseconds past its change time, and the stamp must be unchanged when taken again.
A failure at any step leaves no record and does not fail the capture.

The digest is the capture worker's, as every manifest digest already is.
A worker that lied about it could equally have written matching bytes and a matching manifest, so the restore hash never defended against a dishonest capture worker.
What it defends against is a change after capture, and the stamp detects that without a read.

### Generation root

The broker records the stamp of every artifact of a prepared Generation straight after admitting it.
Launch already uses those admitted handles without re-reading them, because only the installer may write the store.
The root attestation is the root's descriptor digest with its admission stamp, so a restore skips the root read only when the root has not changed since admission.

### Saved overlay

The saved overlay is not hashed at restore today, so there is nothing to skip.
Restore hashes the private head, a reflink clone of the saved overlay the broker made for this restore, and the head stays hashed under this decision.

### Transport

The broker sends each attestation in its own `artifact_attestation` request ahead of `recover` or `fork`.
A worker that predates the request rejects that one packet, and the broker continues with the recovery, which then hashes the input.
An imported bundle may not carry a memory record, because only the broker that published a file on this host can vouch for its stamp.
The stamp type is shared with the Generation admission ledger, and the ledger's file format is unchanged.

## Trust

A record is trusted exactly as far as the directory it describes.
Only the broker can write the checkpoint directory, and only the installer can write the Generation store; the worker receives descriptors and never a path.
Whoever can write a record can already write the file it describes, so a record grants no new authority.
A writer that keeps a descriptor to a recorded file moves the change time with any later write, so its file is hashed.
This relies on the filesystem updating file times on shared-mapping writes, which XFS and ext4 do through their page-write fault handlers and tmpfs does not.
A root process that steps the wall clock backwards can make a later write reuse an earlier change time; root is already inside the broker's trust boundary.
Neither record detects silent media corruption after it is made, which a restore hash would.

## Consequences

Capture pays no extra read: the broker's record costs a sync and two stats.
Every restore and fork of an unchanged memory file and root skips both reads.
The private head and the durable volume are still hashed on every restore.
A checkpoint imported from another host has no memory record and is hashed on every restore.
