# Empty server to running sandbox, executed end to end - 2026-08-30

## Status: current

The full re-audit recorded that the server setup "was not executable as documented on an ordinary
fresh Ubuntu host" and "had no retained end-to-end evidence". This is that run: every step of
[the server setup runbook](../operations/server-setup.md) executed in order on a real host, with
what worked, what took how long, and the two defects the run exposed.

It proves the development path only. It does not certify the host, the Candidate, cleanup, the
jail, networking, or production readiness.

## Host and revision

| | |
| --- | --- |
| Host | `eval-1`, bare metal, Ubuntu 24.04.4 LTS, kernel 6.8.0-138 |
| CPU | Intel Xeon Gold 6138, 80 threads, VT-x |
| Memory | 156 GB |
| Storage | `/srv`, 1.5 TB XFS with `reflink=1` |
| SOMA revision | `bfe10b2c7d149167c623bde8c8196ba02ba273b1` |
| Transport | repository delivered as a git bundle; the host holds no GitHub credential |

## What each step did

| Step | Script | Wall | Result |
| --- | --- | ---: | --- |
| 1 | obtain repository (bundle) | seconds | 1253 files at `bfe10b2` |
| 2 | `setup-host.sh` | about 1 min | ten of ten readiness checks, exit 0 |
| 3 | `build-soma.sh` | 3 m 02 s | cli, guest agent, kernel |
| 4 | `build-fs-tools.sh` | 3 m 50 s | erofs-utils 1.9.4 and e2fsprogs 1.47.0 from pinned source |
| 5 | `prepare-generation.sh busybox:stable-musl` | 4 m 07 s | 424 entries, published atomically |
| 5 | `prepare-generation.sh node:22` | longer, see below | published atomically |
| 6 | run both sandboxes | under 1 s each | see results |

## Results

```text
soma --backend kvm run busybox:stable-musl -- /bin/busybox uname -a
Linux soma-dd67c8b36c5d 6.12.107-soma-v1 #1 SMP PREEMPT_DYNAMIC 2026-08-29T00:00:00Z x86_64 GNU/Linux
real 0m0.773s

soma --backend kvm run node:22 -- /usr/local/bin/node --version
v22.23.2
real 0m0.677s
```

Both are wall-clock times of the whole command, measured by the shell, not internal milestones.
They include process start, resolution, cold boot, the authenticated session, one command, and
cleanup. They are single samples on an otherwise idle host and are not a benchmark.

## The kernel reproduced across two machines

The pinned kernel built on `eval-1` has SHA-256
`f1af3a142fa39916cfac425a01b16b5f328279823533421c9eec3f192c05b746`, byte for byte identical to the
digest recorded in [the kernel build evidence](2026-08-29-x86_64-pvh-kernel-build.md) from a
different host with a different CPU and kernel. That is cross-machine reproducibility observed
rather than asserted. It does not prove reproducibility across toolchain or base-image changes.

## Two defects this run exposed

### The documented bundle command produced an empty clone

The runbook said `git bundle create soma.bundle origin/main`. A bundle made from a remote-tracking
ref carries the objects but no branch a clone can check out, so `git clone` succeeded and left an
empty working tree. The runbook now bundles a real branch and verifies the bundle first.

This was only found by running the documented commands rather than reading them.

### Preparing a large Candidate stalls the entire host

While `node:22` compiled, `eval-1` stopped answering SSH. Nothing had failed: no out-of-memory
kill, no hung task, no soft lockup, no I/O error, no network flap, and no reboot, with the host up
eight days throughout.

The cause was I/O starvation. `/proc/pressure/io` recorded a cumulative `full` stall total of
1,229,894,029 microseconds, about twenty minutes in which every task on the host was blocked on
I/O, `sshd` included. Writing the 1.1 GiB EROFS root and its overlay saturated the device.

Both Candidates completed and published, and the store was left clean, so the work was correct.
The consequence is operational: **preparation is not safe to run beside anything serving
requests**, which is what the architecture already requires when it puts preparation before demand
and off the request path. A production host must throttle or isolate preparation I/O, and this run
is the measured reason why.

## What this does not prove

- One host, one day, single samples. No concurrency, and no distribution.
- No jail. The machine engine is linked into the command line rather than a confined `soma-vmm`.
- No network. The guest device is link down.
- No prepared restore. Every launch here cold boots.
- No certified Generation. Both templates are Candidates launched behind the explicit
  `SOMA_ALLOW_UNCERTIFIED_GENERATION` opt in.
- Nothing here is production-admitted.
