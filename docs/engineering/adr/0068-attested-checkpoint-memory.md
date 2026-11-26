# 0068 - Attested immutable restore inputs

Status: Accepted for implementation

## Context

ADR 0058 has every resident restore and fork hash the checkpoint memory, the Generation root, the private writable head, and any durable volume before it creates the VM.
Those reads cost seconds per GiB and repeat on every restore and every fork of the same unchanged files.
The Generation store already avoids the same cost with its admission ledger: a blob hashed once is recorded by its kernel stamp, and a later admission of the same stamp skips the read.
A stamp is the device, inode, size, and modification and change times, with nanoseconds.
The change time moves on any write, resize, rename-over, or permission change and cannot be set from user space.

## Decision

A trusted broker hands the sealed worker an attestation for the checkpoint memory, the Generation root, and a private head it has just cloned from an attested saved overlay.
An attestation is a digest and the stamp the file had when that digest was known to describe it.
The worker skips a hash only when the descriptor it is about to map has that exact stamp and size and the attested digest equals the digest its record names.
Any other case hashes the file exactly as before, so an absent, damaged, or stale attestation costs a hash and is never a refusal or a skip.
The worker still compares the result with its record, so an attestation can only replace a read and cannot admit bytes the record does not name.

The durable volume is never attested and is always hashed, because a running guest can write it and a stamp proves nothing about its content.
A head created any other way, fresh from a Generation template or by a resume, is not attested and is hashed as before.

### Checkpoint memory and saved overlay

Capture already writes the memory and the saved overlay, syncs each, and reads each back against the manifest's digest before it reports success.
After the directory rename that publishes the checkpoint, the broker records each digest with the file's stamp in `memory.attest` and `overlay.attest`, without reading either again.
A directory rename leaves the stamps of the files inside it unchanged on XFS and ext4.
The record follows a fixed order, with one settle for both files.
Each file is synced first, which cleans and write-protects any shared mapping, so a later write through a mapping a writer kept moves the change time as a write call does.
Each stamp is taken, the records are made only once the clock is at least 50 milliseconds past the newest change time, and each stamp must be unchanged when taken again.
That detects any write made after the settle, by anyone.
It does not stop the capture worker itself, if it kept a writable descriptor or mapping, from writing within the same clock tick as its own last write and before the broker's first stamp.
That gives the worker no new power: the recorded digest is the worker's own sealed digest, so a worker willing to deceive could already write any bytes together with a manifest that names them.
A failure at any step leaves no record and does not fail the capture.

The digest is the capture worker's, as every manifest digest already is.
A worker that lied about it could equally have written matching bytes and a matching manifest, so the restore hash never defended against a dishonest capture worker.
What it defends against is a change after capture, and the stamp detects that without a read.

### Generation root

Admission returns the exact stamp each artifact's content was verified under: the ledger's stamp for an artifact admitted without a read, or the stamp taken before a hash that the artifact still has after it.
An artifact that changed while it was hashed has no stamp, and the ledger no longer records it.
The root attestation is the root's descriptor digest with that stamp, so a restore skips the root read only when the root has not changed since its content was verified.
The parent that admitted the Generation sends the root's stamp to the machine host it launches for a checkpoint restore, beside the verified handles.
A machine host given no stamp, or one that does not decode, has no root attestation and hashes the root.

### Private head

Restore hashes the private head, a reflink clone of the saved overlay; the saved overlay itself is never hashed at restore.
The broker clones the head, then checks in the same step that the saved overlay still has its recorded stamp, so the clone carried the recorded bytes.
The head is created private and never through a link, whether it is reflinked or copied, and it is vouched for only when the head root is a directory of mode 0700 owned by the broker, so no other identity could reach it by name before it was unlinked.
It vouches for the head with the overlay's digest and the head's own stamp, taken after the head is unlinked and before any descriptor reaches the worker.
The worker skips the head hash only when the head descriptor has exactly that stamp and the digest equals the one the manifest expects for the overlay.
Without an overlay record, or when the saved overlay's stamp has moved, the head is hashed as before.
Only the broker holds the unlinked head until the worker has checked it, and any write to the head after the broker's stamp moves the stamp, so the head is then hashed and refused.

### Transport

The broker sends each attestation in its own `artifact_attestation` request ahead of `recover` or `fork`.
A worker that predates the request rejects that one packet, and the broker continues with the recovery, which then hashes the input.
An imported bundle may not carry a record, because only the broker that published a file on this host can vouch for its stamp.
A record left staged by a crash in a locally published checkpoint is a known name, so it never blocks a repeated import.
The stamp type is shared with the Generation admission ledger, and the ledger's file format is unchanged.

## Trust

A record is trusted exactly as far as the directory it describes.
Only the broker can write the checkpoint directory and an unlinked head, and only the installer can write the Generation store; the worker receives descriptors and never a path.
Whoever can write a record can already write the file it describes, so a record grants no new authority.
A writer that keeps a descriptor to a recorded file moves the change time with any later write, so its file is hashed.
This relies on the filesystem updating file times on shared-mapping writes, which XFS and ext4 do through their page-write fault handlers and tmpfs does not.
The broker therefore produces an attestation only for a file whose filesystem reports the XFS or ext4 type; on any other the file is hashed.
A root process that steps the wall clock backwards can make a later write reuse an earlier change time; root is already inside the broker's trust boundary.
Neither record detects silent media corruption after it is made, which a restore hash would.

## Consequences

Capture pays no extra read: the broker's record costs a sync and two stats.
Every restore and fork of an unchanged memory file, root, and saved overlay skips all three reads.
The durable volume is still hashed on every restore.
A checkpoint imported from another host has no memory record and is hashed on every restore.
