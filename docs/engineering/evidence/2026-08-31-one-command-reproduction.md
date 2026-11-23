# One command from an image to a measured launch, and what it refuses - 2026-08-31

Producing a single benchmark number on eval-1 took seven commands in a fixed order. Three of the
ways of getting that order wrong produce a number rather than an error: a Generation compiled but
never captured cold boots and reads about fifteen times slower, a launch at a shape the snapshot
was not captured at is refused before a machine exists so the harness reports zeroes, and a store
prepared against an older wire contract cannot launch at all. None of the three says so.

`scripts/reproduce.sh` is that order as one command, with each precondition checked before it can
fail quietly. This record is its first run on a clean tree, and the failure matrix that is the
actual claim: not that the happy path works, but that every silent path now stops.

## The measured run

`node:22`, one vCPU, 1024 MiB memory, 10240 MiB storage, 25 sequential samples after one discarded
warming launch, on eval-1.

| | |
| --- | --- |
| launches | 25 of 25 returned `v22.23.2` |
| time to first command | p50 **73.95 ms**, min 48.31 ms, max 98.70 ms |
| segment medians | admitted 0.03, machine launched 0.56, ready 21.17, command 52.20 ms |

The command started from an empty scratch root: it built the workspace, the guest agent and both
tools, compiled a 33,512-entry Generation, captured a 1 GiB memory image and a 10 GiB overlay, and
measured. The full transcript is
[`raw/2026-08-31-one-command-reproduction/node22-reproduce-run.log`](raw/2026-08-31-one-command-reproduction/node22-reproduce-run.log);
a `busybox:stable-musl` run at 512 MiB is beside it at p50 28.48 ms.

The `ready` segment at 21.17 ms is consistent with the 22.60 ms this repository already records for
a restore, which is the evidence that the machine was restored rather than booted. The 52.20 ms
command segment is about twice the 27.4 ms this corpus records for `node --version` elsewhere, and
this record does not explain it: it is 25 sequential samples on a host that was not otherwise idle,
and it is quoted as what was measured rather than as a corrected figure. Nothing here supersedes
[the performance findings](2026-08-31-performance-findings.md), which rest on many more samples.

## The failure matrix

Each precondition was broken deliberately and the command rerun. Full output at
[`raw/2026-08-31-one-command-reproduction/failure-matrix.log`](raw/2026-08-31-one-command-reproduction/failure-matrix.log),
harness beside it.

| What was broken | Exit | What it said |
| --- | :--: | --- |
| nothing (control) | 0 | p50 29.32 ms, 3 of 3 |
| `snapshot/` removed from the entry | 1 | has no captured snapshot, so every launch would cold boot and report a number about fifteen times slower with no error, and the `capture_snapshot` command to fix it |
| store captured at 512 MiB, run asked for 1024 | 1 | was captured at `memory_mib=512` but this run asks for 1024; a restore must match the capture shape exactly |
| `kernel/out` repointed at a path that does not exist | 1 | is a dangling symlink to the path, and the two ways to repair it |
| a store from `/srv/soma/sweep` with no stamp | 1 | what it was built against is unknown; treat it as stale and delete it |
| a stamp naming a wire contract this checkout does not have | 1 | names both fingerprints and says the store is stale |
| a command after `--` with no `--expect` | 1 | a command needs text its output must contain |
| `cargo` off the non-interactive PATH with `~/.cargo/env` hidden | 1 | cargo is not on PATH, and what installs it |
| nothing (control, after all of the above) | 0 | p50 29.07 ms, 3 of 3 |

Every refusal happens before a machine exists, so none of them can be mistaken for a slow result.

## How staleness is decided, and what that is worth

A prepared store carries a `.soma-reproduce-stamp` recording the image, the shape, and a
fingerprint of the version constants that decide whether a store can still be launched: the
Generation contract versions, the initramfs layout version, the manifest schema, the snapshot
schema, and the launch-page schema. A store whose stamp disagrees with the checkout in front of it
is refused by name.

A store with no stamp is refused as unknown rather than proved stale. The `SOMACAN` header of a
prepared entry carries only the manifest schema version, which did not change when the layouts that
actually broke those stores did, so nothing in an existing entry distinguishes a usable one from a
stale one. Refusing is the honest answer; claiming to have detected staleness would not be.

## What this does not say

One host, one Generation shape per run, and 25 sequential samples. Concurrency was not measured
here, so nothing in this record bears on the cohort figures elsewhere in this corpus.

The failure matrix proves that each checked precondition refuses. It does not prove the set of
checks is complete: a precondition nobody has hit yet is still unchecked, and the guard against a
cold boot that slips past the snapshot check is a threshold on the `ready` segment, which is a
heuristic rather than a proof of which path a launch took.
