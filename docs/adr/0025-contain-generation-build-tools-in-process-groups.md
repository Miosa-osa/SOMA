# ADR 0025: Contain every Generation build tool in its own process group

- Status: Accepted
- Date: 2026-08-30
- Extends: ADR 0019
- Relates to: `docs/research/generation-compiler.md`

## Context

The Generation compiler runs pinned external tools: the EROFS formatter and checker, and the ext4 formatter, debugger, checker, and dumper.
Each was started as an ordinary child with piped standard input, output, and error, and a deadline enforced by polling `try_wait`.

That containment was incomplete in three ways.
A deadline killed only the direct child, so any descendant it forked kept running and kept the build's output pipes open.
The scoped reader threads then blocked forever on those pipes, so the whole compilation hung past every declared bound.
`wait_bounded` also reported `CompilePhase::FormatRoot` for every timeout, so an overlay, verification, or version-probe failure named the wrong phase in retained evidence.

The implementation audit of 2026-08-29 records this as Priority 0 finding P0.3.
A fixture that forks one descendant and exits immediately reproduced the unbounded hang.

## Decision

Every external tool is started as the leader of a fresh process group through `Command::process_group(0)`.
One signal therefore reaches the tool and every descendant it forked rather than only the direct child.

One supervising thread owns the child for its whole lifetime.
It polls for a normal exit until the deadline, then sends the polite termination signal, waits a bounded termination grace, sends the force signal, waits a bounded force grace, sends the force signal once more so a member that outlived its leader cannot survive, and only then reaps the leader.
Signalling and reaping happen in that thread and in that order, so no signal can reach a process-group identifier the compiler has already released.
A feed failure cancels the supervisor instead of waiting for the deadline.

The standard-input feed keeps running on the calling thread and is bounded by the same supervisor: when the group is terminated the tool's read end closes and a blocked write fails rather than hanging.

Standard output and standard error are drained by two detached readers that retain at most 64 KiB each.
The collector waits at most one bounded capture grace.
An incomplete collection proves a descendant still holds a build pipe, which also proves the group still has a member and its identifier is still reserved, so the compiler forces the group once and collects again.
A build whose tool left descendants holding its pipes fails closed with the invoking phase rather than reporting success.

`TERMINATION_GRACE` is the declared total overrun one invocation may add to its own deadline.
Every error now carries the phase that actually invoked the tool.

## Trust assumption change

`crates/soma-generation` previously declared `#![forbid(unsafe_code)]`.
Sending a signal to a process group has no safe standard-library equivalent, so the crate now declares `#![deny(unsafe_code)]` and one module, `generation::process::control`, carries `#![allow(unsafe_code)]`.
That module contains exactly one `unsafe` call, a `kill` with a negative process-group identifier, with a local `SAFETY` explanation and a test that proves reserved identifiers are never signalled.
No other part of the crate may signal a process, and every other module remains unable to write `unsafe`.
`libc` is added to `soma-generation` under `cfg(unix)` at the version already pinned by the workspace, so the dependency graph gains nothing.
On a platform without process groups the shims are inert, which is correct because the pinned Linux build tools cannot run there at all.

## Verification

A fixture that forks a descendant holding both pipes and exits immediately now fails closed within the deadline plus `TERMINATION_GRACE`, and the recorded descendant is gone afterwards.
A fixture that ignores the polite termination signal and loops is forced within the same bound.
Root, verification, overlay, kernel, and stream phases each report their own deadline failure, and a version-probe failure keeps the phase that asked for it.
A feed failure returns the feed's own phase and kind without waiting for the tool deadline.
Every spawning test checks that the process holds no additional pipe descriptor afterwards.
Before this decision the descendant and stubborn fixtures hung indefinitely instead of failing.

## Consequences

A Generation build can no longer outlive its declared bounds, leak a build process, or leave a build pipe open.
Retained evidence now names the phase that actually failed, so timeout diagnosis no longer misattributes overlay and verification failures to root formatting.
A tool that legitimately needs to leave a background descendant would now fail; no pinned tool in profile v1 does, and a future tool that does would need an explicit decision rather than silent tolerance.

This decision does not add a jail, a namespace, a cgroup, or a seccomp filter around build tools; those remain the builder-isolation work named by the Generation compiler research.
