# The burst harness cannot measure the KVM Backend yet - 2026-08-30

## Status: current

This records the first attempt to run the burst harness against the Linux KVM Backend, on a real
KVM host with a prepared store. It did not produce a performance result. It produced two findings,
one fixed here and one that blocks the campaign until a missing capability exists.

No latency number appears in this document, because none was measured.

## Run identity

| | |
| --- | --- |
| Host | `eval-1`, bare metal, Ubuntu 24.04.4, kernel 6.8.0-138 |
| CPU | Intel Xeon Gold 6138, 80 threads, VT-x |
| Memory | 156 GB, 150 GB available at start |
| Storage | `/srv`, XFS with `reflink=1` |
| Backend probe | `kvm-api-12-vcpu-mmap-12288`, `status: probe_passed` |
| SOMA revision | `a4eea45` |
| Cohort | `node:22`, cold-cache-restore, 3 iterations, concurrency 1 |

The store held a `node:22` Candidate prepared before the timer, and the release binaries were built
through the controlled build step the benchmark contract requires.

## Finding one: measured children could not see the engine settings

The first attempt failed every sample with `launch` exit 76, backend unavailable, in 103 ms of
total wall time. Nothing had booted.

The harness sanitizes the child environment to an eight name allowlist so a measurement is
reproducible and carries no secret. The KVM Backend is configured entirely through environment
variables, so a measured child saw no prepared store, no head directory, and no development opt
in, and refused every launch before doing any work. The harness had only ever been exercised
against the Docker Backend, which needs no such settings.

This is a missing seam rather than a fault in either part, and it is fixed: engine settings are
forwarded by name through the existing reviewed explicit channel, so the value each run used is
recorded in its provenance rather than left implicit in the operator's shell. Widening the
allowlist was rejected because it would admit anything a shell happened to hold.

When the harness was edited in place it refused to run, reporting that the benchmark code changed
after the release build was recorded. That is the anti-gaming control working correctly:
measurement must come from a committed, rebuilt tree. The change was committed, the host updated
to that revision with a clean tree, and the build manifest regenerated before measuring again.

## Finding two: the lifecycle the benchmark needs does not exist yet

With the settings forwarded, wall time rose from 103 ms to 2.32 s, so the run was doing real work.
The per-sample record is unambiguous:

```text
launch:  exit 0     the sandbox booted
exec:    exit 76    backend unavailable
destroy: exit 69
```

The harness drives the lifecycle as three separate `soma` processes: launch, then exec, then
destroy. The KVM Backend holds its live sandbox in an in-process `Option<Live>`. When the launch
process exits, the machine and its authenticated session end with it, so the exec process has
nothing to address.

`soma run`, which performs launch, command, and cleanup inside one process, works on this host and
returned `v22.23.2` from inside a virtual machine earlier the same day. The gap is specific: a
sandbox cannot outlive the command that created it.

This is already recorded as open work rather than a defect discovered here. The public KVM Backend
audit lists "a daemon or API to call, rather than a one shot command; the backend currently tracks
a single live sandbox", and its Stage 6 states that "the single `Option<Live>` is a development
constraint, not the production ownership model".

## Consequence for ticket #14

The admitted burst campaign cannot run against the KVM Backend at this revision. The blocker is a
missing capability, not a harness defect or a configuration mistake, so it cannot be worked around
without changing what is measured.

Two paths exist, and they measure different things.

1. **Measure the one-shot path.** Run concurrency rungs against `soma run`, where one process
   performs create, command, and destroy. This yields real concurrency evidence on this host
   today, including success and cleanup rates under load. It is **not** the ComputeSDK Burst TTI
   boundary and must never be published as though it were, because the timed region and the
   process model both differ.
2. **Build the missing capability first.** A sandbox that outlives its creating command, addressed
   by a keyed ownership table rather than a single in-process slot, with launch returning a handle
   that exec and destroy can name. That is Stage 6 of the audit road and the wiring of
   `soma-hostd`. Only then does the contract profile, `node:22`, 100 iterations, concurrency 100
   released as one burst, timed from before create until `node -v` succeeds inside the sandbox,
   become measurable.

## What this does not prove

- No latency, throughput, or capacity result was produced, and none may be quoted from this run.
- The successful `launch` exit code shows a machine was created; it is not evidence of readiness,
  isolation, or cleanup.
- One host, one cohort, concurrency 1. Nothing here observes concurrent behaviour at all.
- The environment forwarding fix is proven only to the extent that launch then succeeded; no
  complete measured cohort has yet run against this Backend.
