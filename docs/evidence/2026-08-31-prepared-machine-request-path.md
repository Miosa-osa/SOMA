# What a prepared machine removes from the request path - 2026-08-31

## Capability status: Live-proved, diagnostic only

This record measures one segment of one launch: building the machine.
It is not a time-to-first-command result and **no number here may be compared with a benchmark figure or with an eval-1 number**, for the reasons the host section states.

## Observation identity

| Field | Observed value |
|---|---|
| Host | One developer laptop, not a measurement host |
| CPU | Intel Core Ultra 9 275HX, 24 cores, 24 threads |
| Kernel | Linux 7.0.0-30 |
| Storage | ext4, **no reflink** |
| Build | `cargo test` debug profile, unoptimized |
| Generation | `node:22`, compiled and captured on this host, candidate `sha256:e2d84141...` |
| Machine | One vCPU, 1 GiB guest RAM, 256 MiB private head |
| Test | `soma-kvm`, `x86_64_snapshot_restore_prepared`, ten iterations of each arm |

The host has 24 threads and cannot reflink, so it cannot produce a latency result.
What it can measure is the same call made two ways in one process, one after the other, which is what this record is.

## The measurement

Each iteration clones a private head, then times exactly what a Launch would pay for the machine.
The head is cloned **outside** the timed region in both arms, because the head belongs to the Instance either way and preparing a machine cannot remove it.

| Arm | What is timed | p50 ns | min | max |
|---|---|---:|---:|---:|
| On demand | the whole `restore` call | 3,272,392 | 2,693,967 | 4,007,472 |
| Prepared | `Sterile::assign` only | 18,388 | 15,325 | 24,464 |

**Machine construction on the request path falls from 3.27 ms to 18.4 us, a factor of 178.**

Three runs of this test on this host produced ratios of 178, 208, and 211, so the honest
statement is that preparation removes between two and three orders of magnitude from this
segment on this host, and **no single one of those ratios may be quoted alone**. The run
retained below is the one the numbers above come from.
The 3.68 ms that disappears is the memory mapping, the VM, the memory slots, the platform, the five device models, the vCPU and its restored state, the interrupt routing, and the event loop.
None of it depends on which Instance the machine serves, which is why it can be paid before the request arrives.
What remains is the transfer itself: validating and attaching this Instance's private head, installing its context identifier and, where the broker leased one, its frame path, and sampling the readiness challenge.

## The prepared machine works

`a_prepared_machine_reaches_ready_and_runs_one_command` restores a machine before any Instance exists, assigns it a private head and a fresh context identifier, resumes it, completes the authenticated handshake and repair, claims Ready with a receipt bound to the live session transcript, runs `/usr/local/bin/node --version`, receives `v22`, shuts the guest down, and ends with the same open-descriptor count it started with.
So the pool serves a working machine and not only a fast one.

## What this record does not prove

- It is one host, one Generation, and one machine shape, with ten samples per arm and no contention.
- It measures the machine segment only. The handshake, the repair, the readiness probe, and the command are unchanged by preparation and remain the larger part of a launch.
- It does not measure the pool in `soma-local`: it measures the two calls the pool is built on. What the pool adds on the request path is one compare-and-swap and one channel round trip.
- It does not prove capacity admission, jailing, or a separate VMM process, and it assigned no network bundle: the test host runs no broker, so both arms passed no frame path. The `soma-local` pool does hand a claimed machine the leased frame path, and that is unmeasured here.
- The eval-1 figure of 48.0 ms for `machine_launched` at concurrency 100 is a **different segment** of the receipt, measured on a different host under contention. Nothing here may be subtracted from it.

## Retained artifacts

- [`raw/2026-08-31-prepared-machine-request-path/prepared-vs-on-demand.log`](raw/2026-08-31-prepared-machine-request-path/prepared-vs-on-demand.log) - the exact test output above, including every sample
