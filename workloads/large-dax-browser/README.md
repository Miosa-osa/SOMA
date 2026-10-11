# SOMA large-shape DAX image with a live browser

## Purpose

`workloads/large-dax` is the benchmark image for machine contract v2.
This variant adds a browser on top of it, so a large sandbox has a live browser like every default sandbox does.
It is a separate image on purpose: the benchmark image stays byte-identical until a before and after run says the browser costs nothing.
Design and rollout: MIOSA `tasks/soma-browser-2026-10-11.md`.

## What it adds

| Input | Pin |
| --- | --- |
| Base | `BASE` build argument, default `soma-large-dax:3`, built from `workloads/large-dax` |
| Browser | Chrome for Testing `155.0.8059.39`, `chrome-linux64.zip`, SHA-256 `55672d1f392fd3e7b7a08621b6e804e6bcb39d40cf155504abb74b3a021ea8ea`, linked as `/usr/local/bin/chromium` |
| Libraries | the Ubuntu 24.04 packages Chrome links (`libnss3`, `libgbm1`, `libasound2t64`, ...) |
| Fonts | `fonts-liberation`, `fonts-noto-core`, `fonts-noto-cjk`, `fonts-noto-color-emoji` |

Ubuntu 24.04 ships `chromium` only as a snap stub, which cannot run in a guest without snapd, so the browser is Chrome for Testing.

## What it does not change

The base trims apt to `noble` and `noble-updates` with the `main` component so the workload's own `apt-get update` costs two `Hit` lines.
Fonts live in `universe`, so that component is enabled for the install layer only.
The layer restores the base's sources file and refreshes the lists, and the result matches the base: the same sources, six index files, 6060 KiB of lists, and `apt-get update` answers `Hit` twice.
The workload's package list is untouched, so its install is still a no-op.

## Build

```sh
docker build --platform=linux/amd64 -t soma-large-dax:3 workloads/large-dax
docker build --platform=linux/amd64 -t soma-large-dax-browser:1 workloads/large-dax-browser
```

## Measured

Root filesystem 674 MB to 1,483 MB (+809 MB).
`/etc/soma/warm` lists 91 files; none are missing.
