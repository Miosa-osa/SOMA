# 0065 - Import verified checkpoint bundles

## Decision

A trusted host-local caller can install the exact published checkpoint format from private encrypted scratch.
The caller supplies its independent capture receipt and the authorized machine name.
The runtime holds the Instance restore lease, checks the durable owner, and validates the staged receipt, each expected role, and the bytes of every artifact.
The source and destination must be on the configured encrypted mount.
The directory becomes visible at the native checkpoint identity by an atomic rename with no replacement.

A repeat verifies the existing publication and accepts it only when it matches the independently supplied receipt.
The local source machine record retains its normal lifecycle and ownership semantics.
Import is available behind the host-local CLI and creates no new network route.

## Consequences

The normal restore and fork paths continue to check ownership, placement, compatibility and guest readiness.
An imported backup supplies the checkpoint artifacts, while restoring to a new fleet location still needs a durable authorization for its source.
