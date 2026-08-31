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
| On demand | the whole `restore` call | 3,693,944 | 3,189,821 | 6,494,165 |
| Prepared | `Sterile::assign` only | 17,753 | 14,122 | 36,899 |

**Machine construction on the request path falls from 3.69 ms to 17.8 us, a factor of 208.**
The 3.68 ms that disappears is the memory mapping, the VM, the memory slots, the platform, the five device models, the vCPU and its restored state, the interrupt routing, and the event loop.
None of it depends on which Instance the machine serves, which is why it can be paid before the request arrives.
What remains is the transfer itself: validating and attaching this Instance's private head, installing its context identifier, and sampling the readiness challenge.

## The prepared machine works

`a_prepared_machine_reaches_ready_and_runs_one_command` restores a machine before any Instance exists, assigns it a private head and a fresh context identifier, resumes it, completes the authenticated handshake and repair, claims Ready with a receipt bound to the live session transcript, runs `/usr/local/bin/node --version`, receives `v22`, shuts the guest down, and ends with the same open-descriptor count it started with.
So the pool serves a working machine and not only a fast one.

## What this record does not prove

- It is one host, one Generation, and one machine shape, with ten samples per arm and no contention.
- It measures the machine segment only. The handshake, the repair, the readiness probe, and the command are unchanged by preparation and remain the larger part of a launch.
- It does not measure the pool in `soma-local`: it measures the two calls the pool is built on. What the pool adds on the request path is one compare-and-swap and one channel round trip.
- It does not prove capacity admission, jailing, a separate VMM process, a fresh network bundle, or certification.
- The eval-1 figure of 48.0 ms for `machine_launched` at concurrency 100 is a **different segment** of the receipt, measured on a different host under contention. Nothing here may be subtracted from it.

## Retained artifacts

- [`raw/2026-08-31-prepared-machine-request-path/prepared-vs-on-demand.log`](raw/2026-08-31-prepared-machine-request-path/prepared-vs-on-demand.log) - the exact test output above, including every sample
