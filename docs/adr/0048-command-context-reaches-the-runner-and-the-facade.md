# ADR 0048: Carry a command's environment, working directory and user from the runner to the guest

## Status

Proposed.

## Context

The guest protocol has carried a command context since the application frame gained it: up to 64 environment pairs, an absolute working directory, a user, and up to 32 KiB of standard input (`soma_guest::CommandContext`).
The guest agent applies all four.
Nothing above the guest sent them.
`DirectCommand` had no context, the KVM backend built every guest command with `GuestCommand::new`, and the runner refused a `cwd` outright with `RUNNER_UNSUPPORTED_REQUEST`.
A caller that needed a working directory or a variable had to embed it in a shell string.
That is the only reason the control plane could not give a SOMA sandbox the same exec contract it gives a Firecracker sandbox.

## Decision

1. `soma::CommandContext` is a bounded portable value with the guest's own bounds.
   `DirectCommand::with_context` attaches it and refuses a command whose whole encoded size no longer fits one guest record.
   An empty context is the default and changes nothing.
2. The request fingerprint covers the context only when it is not empty.
   Every fingerprint taken before this change, and every command without a context, is byte for byte unchanged.
   A frozen vector pins that.
3. The runner's exec body accepts `cwd`, `env` (an object of string values) and `user`.
   The verdict on a context is the guest constructor's: `GuestCommand::with_context` runs in the runner's admission step, before a machine is asked, so a context the guest would refuse is a 400 or 413 and never costs the sandbox.
   `env` decodes in sorted key order, so one body is always one environment.
   Standard input is not exposed on the runner in this change.
4. The KVM backend lowers the context into the guest command.
   A command without a context crosses to a resident host as the existing `Execute` call.
   A command with a context crosses as a new `ExecuteInContext` call.
   A host built before this change does not name that variant and refuses the line, where an optional field on `Execute` would have been ignored and the command run without its environment.
5. The Docker and macOS backends refuse a command that carries a context with `Unsupported`.
   Running it without the context would be a different command.

## Consequences

- The wire contract between the guest and the host does not change, so no Generation is recertified.
- A new parent talking to a host process started by an older build gets a refusal for a context command and keeps working for every other command.
- A context travels through the idempotency fingerprint, so replaying an operation with a different environment is a conflict rather than a replay.
- The bounds exist twice, in the facade and in the guest crate, as they already do for the command itself.
  A test in `soma-api`, which depends on both, pins them to each other, including a slide across the record boundary.

## Not decided here

Persisted sandbox environment, streaming file transfer, a guest `Stat` and `Rename` request, and incremental exec output are separate decisions (`tasks/soma-guest-api-v2.md` in the MIOSA repository).
