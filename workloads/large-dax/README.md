# SOMA large-shape workload image (machine contract v2)

## Purpose

This directory produces the pinned workload image the [large DAX shape](../docs/) runs: Ubuntu
24.04 with the toolset the install/typecheck workload needs and Node 22 in `/usr/local`. The output
is one OCI image that a Generation is compiled from, and it is the comparison arm's counterpart
rather than a running sandbox.

The image was previously built from scratch outside version control, which meant the benchmark
image could not be pinned or reproduced. It lives here so `soma-large-dax:N` names exact bytes.

## Pinned inputs

| Input | Pin |
| --- | --- |
| Base image | `ubuntu@sha256:534baea6a22c03a63003dbc8dbe78fe34bc0d7e595d9a9dc9834884ff530eb55` (the `docker.io/library/ubuntu:24.04` index digest on 2026-10-09) |
| Node | `22.23.3`, `node-v22.23.3-linux-x64.tar.xz`, SHA-256 `df450af89261115ef9f9e3830c3eeb2cc9213b63c720b1af623cb5dcbe2e02de`, verified in the build |
| Packages | the exact list the workload's `prepare()` installs on apt, plus `xz-utils` for the Node tarball |
| apt sources | `noble`, `noble-updates`; component `main`; no `deb-src` |

## Build

```sh
docker build --platform=linux/amd64 -t soma-large-dax:3 workloads/large-dax
```

The image is a Generation input. To serve it, export it to an OCI layout and compile the Generation
with `prepare_generation`, which is what `scripts/prepare-generation.sh` does for a store.

The Dockerfile deliberately has no `--platform` on `FROM`: the platform belongs to the build
invocation, and a constant there is what the container linter flags.

## Why apt is configured the way it is

The workload's own `prepare()` runs `apt-get update` unconditionally before installing a fixed
package list, and a benchmark harness we do not own is not going to skip it. Official totals include
the prepare phase, so the update has to be made cheap from this side.

Three things, in order of how much they are worth:

1. **Every package `prepare()` installs is already in the image.** The install is then a no-op:
   measured in the image, `apt-get install -y -qq bash build-essential ca-certificates curl git
   python3 python3-setuptools unzip` completes in about 300 ms with `0 newly installed`.
2. **The sources are trimmed to the one suite pair and the one component those packages live in.**
   Each suite and component is another index file, and an index that is not listed is never
   fetched.
3. **The index lists are kept rather than deleted at the end of the build.** `apt-get update`
   compares the `InRelease` it fetches against the lists already on disk and answers `Hit` for an
   index that has not changed, so a machine built from this image downloads no indexes at all:

```
Hit:1 http://archive.ubuntu.com/ubuntu noble InRelease
Hit:2 http://archive.ubuntu.com/ubuntu noble-updates InRelease
```

`Acquire::Languages none` drops the per-language translation indexes, which nothing in this
workload reads, and `Acquire::PDiffs false` takes a full index rather than a patch chain. Neither
was separately measurable on the development machine, where the locale is `C` and the archive
publishes no diffs for these suites; both are here because they are strictly less work.

The measurements that say what the three are worth together are in the two sections below.

## Proven inside a live guest, offline half

`crates/soma-kvm/tests/x86_64_sandbox_boot/apt_prepare.rs` boots a large-shape guest from this
image and reports what it carries:

```
SOURCES=Suites: noble noble-updates;   COMPONENTS=Components: main;   DEB_SRC=0;
LISTS_KIB=6060
PKG_bash=install ok installed  PKG_build-essential=install ok installed
PKG_ca-certificates=install ok installed  PKG_curl=install ok installed
PKG_git=install ok installed  PKG_python3=install ok installed
PKG_python3-setuptools=install ok installed  PKG_unzip=install ok installed
NODE=present
INSTALL_RC=0   INSTALL_MS=80   NEWLY_INSTALLED=0
```

`--no-download` is what makes that an assertion rather than a hope: the install succeeds with the
network forbidden, and names nothing to fetch, so the packages the workload's `prepare()` asks for
are already here.

That gate deliberately does not touch the network. A machine booted by the test harness has no
egress at all, because the device layer puts a link-down placeholder behind the network device when
there is no TAP broker in the process; from there `apt-get update` spends about seven seconds
retrying and then errors, which is a fact about the harness rather than about the image. The update
figure has to come from a sandbox launched by the runner, which is where the workload runs.

## What the update costs, measured in a guest

Two sandboxes launched by the runner, same machine and same session, one on the previous image and
one on this one. The first `apt-get update` is the number that matters, because a fresh sandbox has
only what the image shipped:

| | previous image | this image |
| --- | --- | --- |
| suites / components | `noble`, `noble-updates`, `noble-backports` + `noble-security`; `main universe restricted multiverse` | `noble`, `noble-updates`; `main` |
| `deb-src` entries | 1 | 0 |
| index lists in the image | 4 KiB, deleted at build time | 6060 KiB, kept |
| first `apt-get update` | 2453 ms and 4100 ms in two sessions | **1341 ms** |
| `apt-get install` of the workload's list | 584 ms | **57 ms** |
| what the update fetched | every index of four suites | two `InRelease` files, no indexes |

The first update is three times cheaper and the install is effectively free. The second and third
updates in the same sandbox converge, 667-1710 ms on both, because by then the previous image has
lists too and both are doing the same thing: re-fetching two to four `InRelease` files. That
convergence is the point rather than a disappointment, because this image starts in the state the
old one only reached after its most expensive operation.

## What a host-local apt cache would buy

In this image the only thing the update still fetches is the signed `InRelease` files, about 380 KB.
Measured with `curl` inside the same guest, one of them costs 0.36-0.96 s direct, and they are
fetched in parallel, so roughly 0.4-1.0 s of the remaining 1.3 s is network. A cache on the runner
hosts would serve them in milliseconds and remove most of that, leaving apt's own work at a few
hundred milliseconds.

So it would cut the remainder meaningfully, and it is the only lever left: what can be removed from
the image side has been. Two caveats worth stating. The `InRelease` files have to be revalidated on
every update by design, so a cache cannot remove the cost, only move it to the host. And if the
harness ever stops running `apt-get update` when the packages are already present, the cache stops
being worth anything at all, whereas everything in this image remains useful.

## What this image is not


It is not a security boundary, and it carries no secrets. It is a benchmark image: it exists so the
same bytes are measured on every host, and so the phase the official total includes costs as little
as an image can make it.
