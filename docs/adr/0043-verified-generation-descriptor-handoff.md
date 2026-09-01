# ADR 0043: Transfer verified Generation files to machine hosts

## Status

Accepted.

## Context

Each managed KVM sandbox lives in its own machine-host process.
Reopening and hashing every multi-gigabyte Generation artifact inside that new process added approximately 8.7 seconds to the public create path.
A size-only shortcut restored low latency but allowed same-size content replacement and was therefore unacceptable.

## Decision

The hosted API cryptographically admits every installed Generation before it accepts traffic.
Admission recomputes the manifest identity and every artifact digest, checks the compiler profile and required snapshot artifacts, and retains open handles to those exact files in a process-wide cache.
Semantic artifact certification remains an installer responsibility and must complete before publication into this store.
For each launch, the API opens the retained inode as an independent open file description, then transfers that description to the child over a private Unix socket with one bounded `SCM_RIGHTS` handoff.
Using `File::try_clone`, `dup`, or `SCM_RIGHTS` alone would preserve a shared mutable file offset and is forbidden for launch artifact preparation.
The same socket then carries the canonical manifest and launch request.

The child verifies the manifest identity, hostile decoding rules, compiler profile, snapshot presence, descriptor count and order, regular-file kind, and exact size.
It consumes only the transferred files and never reopens artifact paths.
The handoff receiver reads exactly its fixed header length so it cannot consume a byte from the following launch request.

## Consequences

Expensive content hashing is an API admission cost rather than a per-launch TTI cost.
Path replacement after admission cannot change launch bytes because the kernel open file descriptions are transferred directly.
Restarting the API intentionally repeats cryptographic admission before listening.
Changing an installed Generation requires an API restart or a future explicit atomic cache-reload protocol.
