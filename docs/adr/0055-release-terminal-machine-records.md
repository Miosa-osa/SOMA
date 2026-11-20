# ADR 0055: Terminal machine records are released after a retention window

## Status

Proposed.

## Context

[ADR 0010](0010-durable-managed-lifecycle-state.md) retains terminal operation evidence "according to an explicit bounded replay policy", but no policy was ever written, and neither `StateStore` nor the file store could remove a record.
Every destroyed sandbox, and every launch that failed after it was admitted, left its terminal record in the state root for good.
On host-04 the state root is a 2G tmpfs, and on 2026-10-23 it held 263,782 records, a terminal one about 5.4 KB, and was full ([SOMA#127](https://github.com/Miosa-osa/SOMA/issues/127)).
From then on every create failed its first write with `state_store_failure` (503), and each failure also left an empty Instance directory holding a zero-byte temporary file.

A terminal record is written only after the machine's cleanup completed, so it owes the host nothing.
What it still holds is replay evidence: an exact repeated stop, destroy, or failed launch answers from it, and an inspect answers `MachineStopped` from it.
The public runner never reads it: it mints a fresh operation for every call and answers a repeated destroy from its own memory for ten minutes (`TOMBSTONE_RETENTION`).
The loopback service accepts a caller's operation and Instance identities, so a loopback caller can replay from the record.

## Decision

1. `StateStore` gains `release(instance_id, expected)`, which removes a record only when its durable revision equals `expected` and fails with `Conflict` otherwise, so a release never removes a document newer than the one it judged.
   The file store removes superseded revisions first and the current one last, syncs the removal before it reports it, and removes the Instance directory.
2. The engine owns which records may go.
   `Engine::terminal_census` counts the records and names the terminal ones; `Engine::release_machine` removes a record only when it is terminal at the revision it read, and keeps a record in any other phase.
   A record that cannot be decoded is counted and never released.
3. The retention rule: a terminal record is released once it has been terminal for ten minutes, counted from the first sweep that saw it terminal.
   `soma-api` sweeps every minute on its own thread, whether or not the runner is configured, with facades leased from the request pool: one lease for the census and one per 256 releases, and a pool with nothing to lend skips the sweep.
   A record left by an earlier process is first seen by the new process's first sweep, so after a restart it is released ten minutes later rather than at once.
4. A create that fails before it publishes a revision removes its temporary file and its Instance directory.
   A read that finds an Instance directory with no revision removes it, which clears the directories earlier failures left.
5. The runner's `/healthz` reports `state_records`, the records the store held after the last sweep, or null before the first one.

## Consequences

- The state root holds the live records plus at most about eleven minutes of terminal ones: at 100,000 creates a day that is about 800 records, about 4 MB.
- After the window a released identity reads as never created: inspect, stop, and destroy answer `MachineNotFound`, a repeated operation no longer replays, and a launch with the same Instance identity creates a new machine.
  The runner's answers do not change, because it never read the record after its own ten-minute memory.
  A loopback caller that retries later than ten minutes gets `404` instead of its earlier answer.
- The window is a constant in `soma-api`, not configuration.
- Another process sharing the state root, such as a `soma` command-line run, does not sweep it; only `soma-api` does.
- The first sweep after a fix reaches a store with a backlog reads every record once, and releases the backlog ten minutes later in batches.

## Verification

- `soma` `durable_lifecycle::release`: only a terminal record is released, a live one is kept unchanged, a destroy replays until its record is released, a failed launch's record is released, an undecodable record is counted and kept, and a thousand create and destroy cycles with release leave the store empty.
- `soma-local` `file_store::tests::release`: a release removes every revision and the directory, refuses a stale revision or an absent record, removes a superseded revision a crash left, never removes a record it cannot read, works after a restart, removes an empty leftover directory on first read, and ten thousand cycles on tmpfs roots leave only the shard locks.
- `soma-local` `tests/file_store_full.rs`: on a store where no byte can be written, a create fails and leaves nothing behind, and a release still frees a record.
- `soma-api` `retention::tests`: a record is released only after a whole window, records an earlier process left wait one window, a failed release is retried, an identity that ends again starts a new window, a backlog is released in bounded leases, and a busy pool skips the sweep.
