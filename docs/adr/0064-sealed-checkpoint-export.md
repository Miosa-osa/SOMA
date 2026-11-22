# 0064 - Sealed checkpoint export

## Decision

A host-local backup consumer opens a checkpoint using the complete receipt from capture and the authorized machine name.
The local runtime checks the durable owner and opens the published checkpoint under the same Instance lease used by Restore and Discard.
The checkpoint must reside on the host's configured encrypted mount.
The lease covers descriptor acquisition and can be dropped before streaming because open descriptors pin the original files.

The export accepts only the fixed capture artifacts and returns the receipt separately.
It streams each artifact to a caller-provided writer using bounded memory and checks its length and SHA-256 before reporting success.
A caller must keep an uploaded backup unpublished until every artifact is verified and the manifest has been durably stored.
The backup transport owns encryption, remote object names, authorization for the destination, and the publication transaction.

## Consequences

Off-host backups can survive removal of the local checkpoint during transfer without introducing another on-host copy.
An incompatible host can reject a VM restore while the workspace remains available for a separate cold recovery.
The capture format and the requirements for restoring it are unchanged.
