# SOMA default sandbox image with a live browser

## Purpose

The default SOMA sandbox image is `node:22`.
This directory produces the same image with Chromium and the fonts a page needs, so a live browser works in every default sandbox with no template choice.
Design, measurements, and the rollout runbook live in MIOSA `tasks/soma-browser-2026-10-11.md`.

## Pinned inputs

| Input | Pin |
| --- | --- |
| Base image | `node@sha256:0e5f906573693feaa1e21057ebdcfdb5bd5021f050b2dc7c9deceb629c7da2a8` (the `docker.io/library/node:22` index digest on 2026-10-06, Debian bookworm) |
| Browser | Debian bookworm `chromium` (154.0.8037.92 at the pin date) with every shared library it links |
| Fonts | `fonts-liberation`, `fonts-noto-core`, `fonts-noto-cjk`, `fonts-noto-color-emoji` |

## Build

```sh
docker build --platform=linux/amd64 -t soma-browser-default:1 workloads/browser-default
```

The image is a Generation input, exactly like `workloads/large-dax`: export it to an OCI layout and compile it with `scripts/prepare-generation.sh`, then capture the snapshot at the memory shape the host serves (2048 MiB for the browser-capable default).

## What it costs

Measured on the pinned base, against plain `node:22`:

| | `node:22` | this image |
| --- | --- | --- |
| root filesystem, installed | 1.18 GB | 1.98 GB |
| of which Chromium | | 359 MB |
| of which fonts | | 146 MB |
| of which libraries (Mesa, LLVM, GTK, NSS, ...) | | about 300 MB |

The growth is disk on the lower layer, which is shared by every Instance and paged in on demand.
It adds nothing to the memory a restored Instance holds and nothing to the work a restore does.

## Page-cache warm list

The image carries `/etc/soma/warm`, generated at build time from `ldd` on the Chromium binary.
The guest agent reads every listed file before the capture point, so the snapshot's page cache already holds the browser and its libraries (about 477 MiB in 113 files) and a first browser start reads nothing from disk.
Naming a list replaces the agent's conventional one, so the list also names `node` and `python3`.
No browser process runs at capture.
