# ADR 0047: Execute a declared warm plan before the snapshot capture point

## Status

Proposed.

## Context

A restored Instance starts from captured guest memory.
Whatever was resident when the snapshot was taken is resident again for every Instance at no cost, and whatever was not must be faulted in by each Instance separately.

The guest agent already reads a conventional set of runtime binaries before the repair point, so their file bytes are in the captured page cache.
Reading a binary does not map what executing it needs: the dynamic linker, shared libraries, and the interpreter's own startup data.
On a `node:22` Generation on the production host the first `node -v` in a restored Instance still cost about 55 ms inside the API, while a later call in the same Instance cost a few milliseconds.

An uncommitted experiment on 2026-09-09 executed `node -v` at capture time and measured a one-shot time to interactive of 39 ms.
That experiment hard-coded the commands in the agent, ran them as root with a writable root filesystem, and was never part of a certified Generation.

## Decision

A Template revision may declare a capture warm plan: at most eight commands, each an absolute executable followed by at most fifteen arguments from a narrow character set with no quoting or shell syntax.
The plan has exactly one canonical byte encoding, shared by the compiler and the guest agent through `soma_guest::CaptureWarmPlan`.

The compiler carries the plan in the initramfs as one read-only `warm` entry, which is initramfs layout version 4.
A Generation without a plan keeps layout version 3 byte for byte, so its initramfs digest, manifest, and Generation identity are unchanged.
The manifest already binds the initramfs digest and layout version, and verification re-decodes the archive and requires the plan to be canonical, so the plan is certified and fingerprinted with the rest of the Generation.

The guest agent reads the plan while the initramfs is still its root.
A present plan that cannot be read or decoded powers the machine off, because a verified initramfs cannot hold one.
At the disconnected repair point, after the existing read-through warm and before the flush and the `awaiting launch material` announcement, the agent executes each command once.

Each command is confined so that page cache is the only state it can leave:

- its own session, with the process group and then every other guest process killed and reaped afterwards;
- private mount, IPC, and UTS namespaces in which `/`, `/dev`, `/dev/pts`, `/proc`, and `/sys` are read-only;
- the `nobody` account with no supplementary groups and `no_new_privs`;
- an empty environment apart from a fixed `PATH`, `/` as its directory, and null standard streams;
- a ten second wall-clock budget.

A command that is missing, fails, or overruns is reported on the console and does not fail the boot.

## Security invariants

- The plan runs before launch material exists, so no Instance identity, secret, entropy seed, time sample, network lease, or session is present to observe or capture.
- Identity, entropy, time, and network repair still run after restore for every Instance, unchanged.
- No process started by the plan survives into the snapshot.
- The plan cannot write the root, device, proc, or sys filesystems, create System V IPC objects, or change the hostname.
- The plan is part of the Generation identity; changing it produces a different initramfs digest and a different Generation.

## Consequences

The first command of a runtime the plan executed no longer pays its page-in cost in every Instance.
The page cache captured is larger, which enlarges the snapshot memory image only by pages that were read.
A Generation with a plan has a different identity from the same image without one, so rolling a plan out requires preparing, capturing, and certifying a new Generation.
The plan executes binaries from the image before capture; an image author can already run code in every Instance, and confinement keeps the plan from leaving anything but cached file pages.
