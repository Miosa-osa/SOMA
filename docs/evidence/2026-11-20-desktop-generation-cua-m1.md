# Desktop Generation M1: launch, session and cua driver live; the viewer through the tunnel does not connect

## What was run

`scripts/desktop-proof.sh` on miosa-host-12 (Dell R640, Ubuntu 24.04.5, kernel 7.0.0-43-generic, KVM), at commit `6ac31433` of this branch, against soma-api `1.0.0-alpha.2`.
The Generation is `soma-desktop:small-1` from `workloads/desktop-xfce/`, at 2 vCPU, 4096 MiB and 20480 MiB.
The run finished at 2026-11-21T08:04:27Z.
The samples are in [raw/2026-11-20-desktop-generation-cua-m1](raw/2026-11-20-desktop-generation-cua-m1/).

## Results

| Step | Result | Sample |
| --- | --- | --- |
| Five launches to Ready, each destroyed at once | 70.9, 35.1, 34.8, 35.3, 33.3 ms (the first includes a cold cache); receipt ready 30.6 to 68.1 ms | [measurements.tsv](raw/2026-11-20-desktop-generation-cua-m1/measurements.tsv), [calls.log](raw/2026-11-20-desktop-generation-cua-m1/calls.log) |
| Desktop create, request to Ready | 56 ms | [measurements.tsv](raw/2026-11-20-desktop-generation-cua-m1/measurements.tsv) |
| `soma-desktop-session start` through exec | exit 0, state `ready` | [session-start.json](raw/2026-11-20-desktop-generation-cua-m1/session-start.json) |
| Session milestones after the launcher started | X socket 236 ms, viewer port 241 ms, window manager 589 ms, desktop mapped 1565 ms, cua driver socket 1673 ms, session ready 1674 ms | [session-start.json](raw/2026-11-20-desktop-generation-cua-m1/session-start.json) |
| Host network after the run | unchanged | [residue.txt](raw/2026-11-20-desktop-generation-cua-m1/residue.txt) |
| First frame through the viewer tunnel | **failed**: 1111 attempts, every one `viewer port closed during the WebSocket handshake` | [first-frame.err](raw/2026-11-20-desktop-generation-cua-m1/first-frame.err) |

## What this does not say

- The human viewer path does not work yet.
  Inside the guest KasmVNC answers a local WebSocket upgrade with 101, but through `GET /v1/sandboxes/{id}/tunnel/6080` it logs `write: Broken pipe` about 10 ms after accept.
  The same failure happens with the `1.0.0-alpha.1` build, so it is not a regression of `alpha.2`.
  It is tracked as [issue #176](https://github.com/Miosa-osa/SOMA/issues/176).
- No first frame was captured, so no frame latency is claimed.
- The cua driver socket being up is measured; no agent action through it is measured here.
- One host, one run, at one shape.
