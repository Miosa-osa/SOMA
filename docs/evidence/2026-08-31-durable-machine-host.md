# A `soma machine` sandbox that survives its launching process - 2026-08-31

Measured on eval-1 at `6234c00`, KVM backend, busybox at one vCPU, 1024 MiB of memory and
2048 MiB of storage, against a Generation compiled and captured by the same binary at that shape.
Raw envelopes and result streams are under
[`raw/2026-08-31-durable-machine-host/`](raw/2026-08-31-durable-machine-host/).

This supersedes the finding in
[the machine surface does not survive its process](2026-08-31-durable-machine-across-processes.md),
which recorded the same commands failing on the code this branch replaced.

## What was wrong

A KVM sandbox is a set of descriptors, a guest memory mapping, a vCPU thread, and a Noise
session. The Backend held all four in one field of a struct that lived in the command's own
process, so a machine died with the command that launched it. `machine launch` returned
`status: ok` with an Instance identity, and a separate `machine exec` naming that identity was
refused with `backend_unavailable`.

## What now happens

A managed Launch starts a host process that holds exactly one machine and is addressed by the
Instance identity that names it, over a socket at `<state-root>/machines/<instance-id>.sock`. The
host binds that socket before it builds anything, runs the unchanged resident lifecycle, reports
what the launch established on its standard output, and then answers execute, inspect, and
cleanup over the socket until the machine is released.

`soma run` and `soma_run` are untouched. Each holds its own machine in its own process for the whole operation and
releases it before returning, so no second process appears on the path every performance figure
in this repository was measured on. One `soma run` at this shape still reached Ready in 20.5 ms
and released gracefully ([`one-shot-run.json`](raw/2026-08-31-durable-machine-host/one-shot-run.json)).

## One sandbox, five separate processes

Every line below is a separate `soma` invocation.
[`raw/.../lifecycle/`](raw/2026-08-31-durable-machine-host/lifecycle/) retains each JSON envelope.

| Process | Command | Exit | Result |
| --- | --- | ---: | --- |
| 1 | `machine launch --instance-id a148...f77` | 0 | `state: ready` |
| 2 | `machine exec ... -- /bin/sh -c 'echo persisted-by-the-first-process > /tmp/proof.txt; ls -l /tmp/proof.txt'` | 0 | `exited: 0` |
| 3 | `machine exec ... -- /bin/cat /tmp/proof.txt` | 0 | stdout `persisted-by-the-first-process\n` |
| 4 | `machine inspect ...` | 0 | `state: ready` |
| 5 | `machine destroy ...` | 0 | `terminal_status: destroyed`, cleanup method `forced` |

Process 3 read back what process 2 wrote. The two are separate operating-system processes with no
shared memory, so the guest filesystem state between them was held by the machine itself, which
outlived both.

## The contract benchmark

The burst harness spawns a separate `soma` process for launch, for exec, and for destroy, which
is why it had reported 0 of 100 on every run in this repository's history.

```
python3 -m benchmarks.local_alpha.burst run --experiment-class warm-cache-restore --backend kvm \
  --image busybox:stable-musl --iterations 100 --concurrency 100 --vcpus 1 --memory-mib 1024 \
  --storage-mib 2048 --prepared "the Generation store, the host page cache, and the release build" \
  --build-manifest <abs> --soma-bin <abs> --soma-mcp-bin <abs> --results <abs> -- /bin/echo soma-ok
```

Three independent hundred-way runs at the same build:

| Run | Attempted | Command succeeded | Cleanup complete | TTI p50 | TTI p95 | Wall |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 100 | **100** | **100** | 1597.92 ms | 3733.20 ms | 6.07 s |
| 2 | 100 | **100** | **100** | 1367.14 ms | 2745.13 ms | 5.45 s |
| 3 | 100 | **100** | **100** | 1305.86 ms | 4045.48 ms | 6.56 s |

Every one of the nine hundred processes those runs spawned exited zero: three hundred launches,
three hundred execs, three hundred destroys, no other exit code anywhere. The build manifest
records `git_revision 6234c00` and `worktree_clean: true`. The stage table below is from run 1.

The time-to-first-command boundary spans two process spawns by design, and at a hundred concurrent
slots most of it is not the machine. Stage medians from the retained receipts:

| Stage | p50 | p95 |
| --- | ---: | ---: |
| launch: workload resolution | 0.00 ms | 0.00 ms |
| launch: admission | 447.46 ms | 700.56 ms |
| launch: machine creation | 0.00 ms | 0.00 ms |
| launch: readiness | 55.41 ms | 95.05 ms |
| exec: command execution | 14.21 ms | 17.08 ms |
| destroy: cleanup | 965.91 ms | 1141.33 ms |
| harness: process and transport overhead | 1076.33 ms | 2970.00 ms |

Readiness is the whole of starting the host process, restoring the machine, and reaching an
authenticated session: 55.41 ms at the median under a hundred-way burst, against 20.5 ms for the
single-process `soma run` doing the same restore unloaded and without a second process. Admission
is the durable state write and cleanup is the release; both are the file-backed state store under
a hundred simultaneous callers rather than the sandbox. Neither is compared against a prior figure
here, because neither path completed before.

## What destroy releases

A machine's private overlay head is created under `SOMA_HEAD_DIR` and immediately unlinked, so the
host process holds the only reference to it. Directly observed on a live host: file descriptor 10
is `/srv/soma/dm-heads/<instance-id> (deleted)`, the process holds 48 descriptors and 8 threads,
and its resident set is 17.7 MiB. A released Instance therefore has to leave both no file in the
head directory and no process holding a deleted one.

After the five-process lifecycle above and again after each hundred-way run:

| Check | Observed |
| --- | ---: |
| Entries in `SOMA_HEAD_DIR` | 0 |
| Sockets under `<state-root>/machines` | 0 |
| `soma machine-host` processes | 0 |
| Mount entries naming the scratch tree | 0 |

## The MCP server, which is the surface an agent actually holds

`soma-mcp` opens a fresh runtime for every tool call, so a sandbox `soma_launch` created had
exactly the lifetime of that one call. It now asks for a hosted machine for the same reason the
command line does, and because a launch starts its host by re-executing the binary it is already
running, the MCP executable serves that host too, ahead of argument parsing and off the tool
surface entirely.

Three separate `soma-mcp` server processes over one sandbox
([`mcp/three-server-processes.txt`](raw/2026-08-31-durable-machine-host/mcp/three-server-processes.txt),
driven by [`three-server-processes.py`](raw/2026-08-31-durable-machine-host/mcp/three-server-processes.py)):

| Call | Server process | Result |
| --- | ---: | --- |
| `soma_launch` | 957875 | `state: ready` |
| `soma_exec` writing `/tmp/two.txt` | 958101 | stdout `wrote\n` |
| `soma_exec` reading it back | 958188 | stdout `written-by-the-first-mcp-process\n` |
| `soma_destroy` | 958188 | `state: destroyed` |

No call reported an error and no socket survived the destroy.

## Stop and destroy are different terminations, and the evidence says which happened

A forced destroy ends the machine without asking the guest; a graceful stop asks and waits. The
portable contract refuses evidence naming the wrong one, which is what `machine destroy` hit the
moment it could reach a machine at all. Both were then observed, on separate sandboxes:

| Command | Exit | Terminal status | Cleanup method |
| --- | ---: | --- | --- |
| `machine stop` ([`graceful-stop/stop.json`](raw/2026-08-31-durable-machine-host/graceful-stop/stop.json)) | 0 | `stopped` | `graceful` |
| `machine destroy` ([`lifecycle/5-destroy.json`](raw/2026-08-31-durable-machine-host/lifecycle/5-destroy.json)) | 0 | `destroyed` | `forced` |

`graceful` is reported only because the guest actually halted on its own; a guest the host had to
end would have been reported as `graceful_then_forced`. An `exec` naming a stopped Instance is
refused with `state_conflict`, and the stopped sandbox left no socket behind.

## When the client is gone

A host that has been killed outright is not a machine anybody can reach, and the surface says so
rather than reporting one. With the host process sent `SIGKILL`:

| Command | Exit | Reported |
| --- | ---: | --- |
| `machine exec` | 76 | `backend_unavailable` |
| `machine destroy` | 0 | `terminal_status: destroyed`, cleanup machine `not_owned` |

Destroy is honest in both directions: it does not claim to have released a machine this process
never held, and it still closes the durable record. The stale socket is removed by the first
lookup that finds nothing behind it. No head and no socket survived the kill, because the killed
process was the only holder of both.

### A client that never comes back

The harder case is a client that dies holding a live sandbox and never destroys it. A host whose
machine nothing has addressed for half an hour asks its own socket to shut down, which ends the
serve loop on the one thread that owns the machine and releases it there.

One sandbox was launched and then deliberately never addressed again
([`abandoned-machine/idle-release.log`](raw/2026-08-31-durable-machine-host/abandoned-machine/idle-release.log)),
with the host process, the head directory, and the socket sampled once a minute:

```
launch exit=0 at 2026-08-31T09:37:03+00:00
2026-08-31T10:05:06+00:00 minute=28 hosts=1 heads=0 sockets=1
2026-08-31T10:06:06+00:00 minute=29 hosts=1 heads=0 sockets=1
2026-08-31T10:07:06+00:00 minute=30 hosts=0 heads=0 sockets=0
REAPED at minute 30
```

The machine stayed up for twenty-nine minutes of being ignored and was gone at thirty, leaving no
process, no head, and no socket. One sample, at the compiled ceiling; no other ceiling was tested.

One further sequence is worth naming: an `exec` that failed against a dead host moves the durable record
to a terminal phase, and a `destroy` after that is refused with `state_conflict` and exit 69
rather than succeeding ([`killed-host/exec-first/destroy.json`](raw/2026-08-31-durable-machine-host/killed-host/exec-first/destroy.json)).
That is the durable state machine's existing behaviour rather than anything the host introduced,
and it is recorded here because it is the one path on which destroy does not return zero.

## What is not proved here

- The host is an ordinary child process. It is not jailed, and `soma-vmm`, which is, still does
  not host a machine. See the note below.
- `soma-api` still opens its runtime without asking for hosted machines, so the HTTP surface keeps
  the in-process lifetime it had. It is a single long-lived process, so its sandboxes outlive each
  request already; nothing here changes or proves it.
- Nothing here was measured on a second host, and every figure is one run.

## The jail is still ahead of this

The intended long-run home for a resident machine is the jailed `soma-vmm` worker, which is
live-proved as a process and holds no filesystem at all: it receives sealed descriptors and
refuses to serve anything when its attestation does not describe a jail. The KVM lifecycle reaches
its Generation store, its head directory, and `/dev/kvm` by path, so moving it inside that jail
means giving it directory descriptors and `openat` throughout, which is a larger change than this
one and does not change what a caller can do. This document's host is that same one-machine,
one-process shape without the jail around it.
