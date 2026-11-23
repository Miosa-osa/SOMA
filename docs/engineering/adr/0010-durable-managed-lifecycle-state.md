# ADR 0010: Durable managed lifecycle state

- Status: Accepted
- Date: 2026-08-28

## Context

A managed Machine outlives the command-line process that launches it.
An MCP server can also restart while its Machine remains alive.
Keeping the active Machine record only in process memory would make a later execute, inspect, stop, or destroy request forget the workload identity, requested shape, human name, launch evidence, and retry history that the facade needs to produce a truthful receipt.

Reconstructing caller intent from a backend inspection is not sufficient.
A backend may observe an Instance and some effective resource values, but it cannot infer the original requested values, the canonical request fingerprint, an optional name, or the exact result of an operation that completed before a process crash.

Managed operations also cross crash windows.
A process can exit after it changes a VM but before it records the outcome, and two processes can attempt conflicting operations against the same Instance concurrently.

## Decision

The portable facade uses an explicit durable state-store seam for managed Machine lifecycle state.
The durable key is the globally unique Instance ID, never the optional human-readable Machine name.

Stored records are bounded, versioned, validated before use, and advanced with revisioned compare-and-swap operations.
The store provides create-if-absent, load, and compare-and-swap semantics so two callers cannot both acquire the same state transition.
Corrupt, oversized, unsupported-version, missing, or conflicting records fail closed.

The facade records intent before invoking a backend side effect.
The lifecycle includes write-ahead states for launching, active, executing, terminating, and terminal outcomes.
An active record retains the resolved workload identity, requested Machine shape, optional Machine name, verified launch evidence, and the operation evidence required by later receipts.

An interrupted launch is never published as Ready from state alone.
Recovery must reconcile the exact owned backend Instance and either prove the completed launch contract or perform rollback cleanup.
An interrupted execute leaves command completion uncertain, so the Machine becomes unavailable for another execute until owned cleanup or an explicit recovery contract proves safety.
An interrupted stop or destroy resumes idempotent cleanup from its terminating record.

Terminal operation evidence is retained according to an explicit bounded replay policy.
A repeated operation ID with the same canonical request replays its exact result while that bounded result is retained.
After captured output is evicted, a compact tombstone preserves the operation ID, fingerprint, and validated receipt metadata so the facade returns an explicit replay-unavailable result without repeating the command.
Reuse of an operation ID with different input is always a conflict.
Reaching the tombstone capacity rejects further execution and requires Machine replacement rather than forgetting an old operation.
The facade never silently repeats an uncertain guest command.

The repository provides a memory store only for process-local tests and deliberately labeled ephemeral use.
The local CLI and stdio MCP runtime use a durable file-backed store with an explicit root, per-store locking, bounded documents, synchronized temporary writes, atomic replacement, and restrictive permissions where the host supports them.
A fleet control plane may implement the same compare-and-swap contract with a durable database without changing Machine use-case semantics.

## Alternatives considered

### Keep state only in the Engine process

This option was rejected because a normal CLI invocation exits after launch and an MCP server can restart.
The next process would either reject a real owned Machine as unknown or operate without enough evidence to construct a truthful receipt.

### Reconstruct all state from backend inspection

This option was rejected because observed backend state is not a substitute for caller intent or durable operation history.
Inventing requested shape, name, fingerprint, or prior results from runtime metadata would weaken the evidence contract.

### Put state only in OCI labels

This option was rejected because labels are backend-specific, size-limited, and do not provide a portable atomic revision or replay contract.
Labels remain useful for independent ownership verification but are not the source of truth for facade state.

### Allow duplicate execution after a crash

This option was rejected because an agent retry could repeat a non-idempotent command.
Uncertain execution invalidates the Machine until cleanup or a future authenticated recovery protocol proves the command outcome.

## Consequences

Managed lifecycle is safe across ordinary CLI and MCP process restarts when a durable store is configured.
The state store becomes security-sensitive control-plane data and must be included in permissions, corruption, concurrency, backup, and cleanup testing.

The store does not hold guest memory, guest files, provider credentials, or persistent workspace data.
It records bounded control evidence and lifecycle intent only.

One-shot run remains a self-contained transaction and does not require a durable Machine record after verified cleanup.
Machine resizing remains replacement-based, so a new shape creates a new Instance and a new durable record.

## Verification

Contract tests must cover create conflicts, revision conflicts, restart recovery, record corruption, unsupported versions, oversized records, and exact operation replay.
Concurrency tests must prove that only one execute or termination transition can own an active Instance at a time.
Crash-window tests must cover interruption before and after each backend side effect and before and after each durable transition.
Cross-platform tests must cover the file store on Linux, macOS, and Windows.
Backend tests must still verify independent ownership before every mutation even when the durable state record is valid.
