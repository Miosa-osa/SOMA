# The KVM Backend serving one sandbox through the public command line - 2026-08-30

## Status: current

This is the first sandbox SOMA has launched through its own public surface on the Linux KVM
Backend. Every earlier x86_64 result was produced by an ignored test in `crates/soma-kvm` driving
the machine directly. This run goes through the portable facade: `soma run` resolves a workload,
launches a machine, executes one command, and cleans up.

It proves the first vertical slices of [the KVM backend integration](../research/kvm-backend-integration.md),
which that document orders as Ubuntu cold boot and file read, authenticated command, Generation
restore, private disk, isolated network, full lifecycle, prepared worker, then the 100-way burst.
Cold boot, the authenticated command, and the private disk are what run here. **It is not the
adapter that document specifies**, and the section below says exactly what it does not do.

## Run identity

| | |
| --- | --- |
| Commit | `08e4d4526d877b71b23c6651d230729adbd39847` |
| Command | `soma --format json --backend kvm run node:22 -- /usr/local/bin/node --version` |
| Host | Linux 7.0.0-30-generic x86_64, Ubuntu 24.04.4 LTS, in a container with `--device /dev/kvm` |
| Build | release |
| Request fingerprint | `sha256:6a941ba8cbf7a7ff41662dcab31f211c40cb142e438b8b5bc495f919c38ab98a` |
| Image manifest | `sha256:87a4f951f28b85d189df365d24c479d3bdb70be77c1ff5c9029db2ef67e251ac`, `linux/amd64` |
| Instance | `89db112753324c3e890ef78b74381aa5` |
| Retained receipt | [`raw/2026-08-30-kvm-backend-cli/receipt.json`](raw/2026-08-30-kvm-backend-cli/receipt.json) |

The Generation was prepared before the run, as the request path requires: the Backend resolves
against a prepared store and never acquires an image or compiles a Generation while serving a
request.

## Result

```
status: ok
stdout: v22.23.2
exit:   0
```

The guest is a `node:22` Generation. The command ran the interpreter inside the machine and
returned its version, so the result is the workload's own output rather than a host-side probe.

## What the receipt reports

| Field | Value | Meaning |
| --- | --- | --- |
| `backend` | `linux_kvm` | |
| `isolation` | observed `hardware_virtual_machine` | not asserted from configuration |
| `preparation` | observed `on_demand` | this run cold booted its own machine; no worker was prepared |
| `digest_binding` | observed `launch_enforced` | the machine was built from the exact prepared artifacts |
| `effective_shape` | observed 1 vCPU, 1024 MiB; storage not verified | |
| `effective_network` | observed detached, egress denied, no addresses | the guest's one device is link down |
| `cleanup` | machine, memory, storage, guest authority all `complete`; network `not_owned` | nothing host-side backs the link-down device |

## Timings

Milliseconds from the facade accepting the request:

| Milestone | Elapsed |
| --- | ---: |
| `accepted` | 0.00 |
| `workload_resolved` | 0.00 |
| `admitted` | 0.00 |
| `machine_launched` | 305.98 |
| `ready` | 646.00 |
| `command_started` | 646.01 |
| `command_finished` | 732.15 |
| `cleanup_started` | 732.15 |
| `cleanup_finished` | 796.51 |

This is one sample on a busy host. It is not a benchmark, and it must not be compared with the
warm restore figures, which measure a restored Instance rather than a cold boot, or with any
competitor's creation-stage number.

Resolution is effectively free because it reads a prepared store rather than building anything.
The window before `machine_launched` covers opening the prepared artifacts, giving the Instance
its private overlay head, and creating the machine. A separate measurement of the head path found
reflink at 136 to 142 ms against a copy at 169 to 197 ms across three runs each, so the head is a
part of that window rather than the whole of it, and the rest is unattributed.

## What this does not prove

Measurement limits:

- One sample, one host, one image, one shape, one day. It says nothing about concurrency; the
  burst campaign is ticket #14.
- The timings are not a benchmark and the window before `machine_launched` is only partly
  attributed.

Everything the specified adapter requires and this run does not do:

- **No snapshot restore.** Launch is required to restore a snapshot; this cold boots, which is why
  it reaches Ready in roughly 646 ms rather than the restore figures measured separately.
- **No certified Generation.** Resolve is required to select an installed certified Generation.
  Certification does not exist, so this resolves a Candidate and the receipt's `generation_id` is
  null rather than naming an identity no gate has produced.
- **No capacity admission, and no durable operation ownership** recorded before an external
  effect.
- **No retry semantics.** A conflicting request fingerprint against the same OperationId is not
  rejected, because retries are not implemented.
- **No prepared worker and no sterile bundle.** Every launch creates its own machine, which is why
  `preparation` is `on_demand`.
- **No network lease and no activation.** The device is link down and no packet leaves the
  machine, which the receipt reports rather than hides.
- **No separate Stop and Destroy** with independently reported dispositions, and no reconcile
  after restart.
- **No jail.** The real `soma-vmm` is not wrapped by `soma-jail` in this path.
- The Generation was prepared out of band, so this exercises no preparation path; none exists.
- Nothing here is production-admitted, and no signed admission report exists.
