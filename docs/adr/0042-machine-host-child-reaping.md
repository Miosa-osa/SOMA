# ADR 0042: API process owns machine-host exit collection

## Status

Accepted.

## Context

Hosted KVM launch creates one machine-host child and transfers sandbox lifecycle to that process after an authenticated startup handshake.
Dropping Rust's `Child` handle does not collect the eventual Unix exit status.
A long-running API therefore accumulated zombie children after successful sandbox destruction even though guest resources were cleaned correctly.

## Decision

The API process owns exit collection for every machine-host child it launches successfully.
After the startup handshake succeeds, launch transfers the `Child` handle to one process-wide reaper thread.
The adoption queue is bounded at 1,024 handles.
When the queue is full, adoption waits only until the live reaper drains one queue slot and takes ownership, never until the machine exits.
When the queue is disconnected, adoption terminates and collects the child instead of returning an Instance whose exit nobody owns.
The reaper polls running children every 10 ms, removes collected exits, and waits synchronously when `try_wait` returns an indeterminate error.

The reaper lives for the API process lifetime.
It does not own guest cleanup, durable Instance state, or machine-host termination.
Those responsibilities remain in the lifecycle and reconciliation paths.
Normal API process exit lets the operating system reparent any still-running machine hosts, while durable reconciliation remains responsible for recovering their Instance state after restart.

## Consequences

Successful create and destroy cycles no longer leave zombie machine-host processes under a persistent API parent.
Ordinary launch cannot stall on exit collection while the bounded queue accepts ownership.
Exceptional queue saturation can delay adoption by one reaper drain interval, while reaper loss terminates the affected machine before Launch can report it ready.
Qualification must inspect the API parent after a completed burst and prove that no exited children remain.
