# Eight unrelated OCI images compiled, captured, restored, and asked a question - 2026-08-31

Everything else in this corpus was proved with `node:22` and `busybox`. A sandbox has to accept
whatever image a caller names, so this record exists to say whether the pipeline is general or
whether it is two images that happen to work.

Eight images were walked end to end on eval-1: compile the OCI layout into a Generation, capture
its snapshot, restore it, and run one command that only succeeds if the workload is really inside
the machine. The set was chosen to be awkward rather than representative: three glibc
distributions, one musl, one security distribution with a very large root, and three language
runtimes.

## Result

Every stage of every image succeeded. Eight of eight prepared, eight of eight captured, eight of
eight ran their probe and returned the expected text.

| Image | Probe | What came back |
| --- | --- | --- |
| `ubuntu:24.04` | `/bin/cat /etc/os-release` | `Ubuntu 24.04.4 LTS` |
| `ubuntu:22.04` | `/bin/cat /etc/os-release` | `Ubuntu 22.04.5 LTS` |
| `debian:12` | `/bin/cat /etc/os-release` | `Debian GNU/Linux 12 (bookworm)` |
| `alpine:3.20` | `/bin/cat /etc/os-release` | `Alpine Linux v3.20`, `3.20.10` |
| `python:3.12` | `/usr/local/bin/python3 --version` | `Python 3.12.14` |
| `golang:1.23` | `/usr/local/go/bin/go version` | `go1.23.12 linux/amd64` |
| `node:22` | `/usr/local/bin/node --version` | `v22.23.2` |
| `kalilinux/kali-rolling` | `/bin/cat /etc/os-release` | `Kali GNU/Linux Rolling`, `2026.3` |

Three of the eight recorded a time to first command that survived into the retained file:
`node:22` at 61.5 ms, `python:3.12` at 69.9 ms, and `golang:1.23` at 70.2 ms. Those are one sample
each, taken while other work ran on the same host, and they are the wrong instrument for a latency
claim. They are here because they say the restored machine ran a real runtime, not because they
say how fast it did.

The samples and the harness are retained at
[`raw/2026-08-31-image-matrix/`](raw/2026-08-31-image-matrix/).

## What this does not say

The artifact records no run revision, so this record's retention point is the file rather than a
checked run identity, and no claim-ledger row rests on it.

Each image was run once. A single success does not distinguish an image that works from an image
that usually works, and nothing here checks cleanup: no head directory, process table, or mount
list was inspected afterwards, which is exactly where a leak would hide. The `run` details for the
five `/etc/os-release` images were truncated by the harness at two hundred characters and their
timing fields were lost with the truncation, so only three of the eight carry a figure at all.

The set is also not the hard case. Every image here has a conventional root and an executable at a
predictable path. An image whose entrypoint is a daemon, one with an unusual layout, or one large
enough to change the compile step's behaviour are all untested.
