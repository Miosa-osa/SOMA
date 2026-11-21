# SOMA desktop: Xfce, KasmVNC and the cua driver

## Purpose

This directory turns a Linux desktop image into a SOMA Generation an agent can drive.
A desktop is not a new kind of machine: it is an ordinary Generation (the same kernel, guest agent, devices and tunnel as every sandbox) at a desktop shape, whose session is started after the machine is Ready.
[The M1 evidence record](../../docs/evidence/2026-11-20-desktop-generation-cua-m1.md) has the measurements: launch, session start and the cua driver work; the human viewer through the tunnel does not connect yet.

| Piece | What it is |
| --- | --- |
| X server and human viewer | KasmVNC 1.4.0 (`Xkasmvnc :0`), RFB over WebSocket and its own web client on guest port 6080 |
| Desktop | Xfce 4 with `xfce4-terminal` |
| Agent hands and eyes | `cua-driver` 0.55.0 (MIT), running as the desktop user as `cua-driver serve` on its owner-only Unix socket |
| Accessibility | the AT-SPI bus from `at-spi2-core`, which the driver reads trees from |
| Session start | `soma-desktop-session start`, one command through exec |

## Pinned inputs

| Input | Pin |
| --- | --- |
| cua-driver | `cua-driver-rs-0.55.0-linux-x86_64.tar.gz`, SHA-256 `fd863defadb13573d53fd327e7d09268460a658224651329cb0b857b0c8752ac`, the value in the release's `SHA256SUMS` |
| KasmVNC (public base) | `kasmvncserver_noble_1.4.0_amd64.deb`, SHA-256 `12bac6014149c5fdee75f0d403785aaa3e5dd4ea222de73253a5d4181bc9567e` |
| Ubuntu (public base) | `ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517` |

cua-driver 0.55.0 is the newest version with published release binaries, `SHA256SUMS` and Sigstore bundles.
The `cua-driver-rs-v0.56.0` tag exists upstream but has no release assets, so there is no binary of it to verify.
Only the MIT driver binary, its licence and its third-party notices are installed; the build fails if any `cua-spacesd`, `cua-relay` or `cua-perception` file appears in the image.

## Build

The stage in `Dockerfile` needs a base that already has Xfce 4, `xfce4-terminal`, `dbus-x11` and KasmVNC 1.4.0.
Two are supported, and the stage is the same for both:

```sh
# a public base, from this directory alone
docker build -f workloads/desktop-xfce/base.Dockerfile -t soma-desktop-base:1 workloads/desktop-xfce
docker build --build-arg BASE_IMAGE=soma-desktop-base:1 -t soma-desktop-xfce:1 workloads/desktop-xfce

# or an existing desktop root filesystem (an ext4 file), imported first
workloads/desktop-xfce/import-rootfs.sh <rootfs.ext4> desktop-rootfs:1
docker build --build-arg BASE_IMAGE=desktop-rootfs:1 -t soma-desktop-xfce:1 workloads/desktop-xfce
```

Only classic-builder instructions are used, so neither needs BuildKit.

Then compile and capture the Generation, as root on the KVM host:

```sh
scripts/desktop-generation.sh soma-desktop-xfce:1 soma-desktop:small-1 <store> <fs-tools> small
```

`small` is 2 vCPUs, 4096 MiB and 20480 MiB writable, the platform's small computer.
`medium` (4 vCPUs, 8192 MiB) is accepted by the script but has not been captured or run.

## How the session runs

The snapshot holds no running process (the capture point forbids it), so the desktop starts after Ready:

1. `soma-desktop-session start` runs as root through exec.
   It mounts cgroup v2 if the guest has not, moves itself into the cgroup `/soma-desktop`, and writes `cgroup:/soma-desktop` to `/run/soma/survivors`.
   The guest agent kills every process when a command ends except the survivors; a desktop cannot be named by process id, group or session, because a terminal's shell and D-Bus activated services start sessions of their own, so the cgroup line is what keeps the desktop alive.
2. It starts `session.sh` as the desktop user with `setsid`, which brings up, each waiting for the one before: the session D-Bus at `$XDG_RUNTIME_DIR/bus`, `Xkasmvnc :0` with the viewer on 6080, the accessibility bus, Xfce (until the panel and the desktop window are viewable), and `cua-driver serve`.
3. It waits for `session_ready` and prints every milestone as JSON (guest clock, nanoseconds).

`soma-desktop-call` runs one driver command as the desktop user, for example `soma-desktop-call doctor` or `soma-desktop-call call get_desktop_state '{"max_image_dimension":0}'`.
Per-Instance identity is the guest agent's: it writes `/etc/machine-id` for every Instance, and the image links `/var/lib/dbus/machine-id` to it.

## cua-driver configuration

- `standard` permission mode with the capability manifest `session/cua-capabilities.yaml` as a ceiling: the tools the platform's desktop contract maps onto, the desktop display, the windows of the terminal, panel and desktop, and one writable directory for screenshots (`/run/soma-desktop/captures`).
  Recording, scripting, browser control and process termination are not in it.
- Telemetry and the update check are off for every caller: `/usr/local/bin/cua-driver` exports `CUA_DRIVER_RS_TELEMETRY_ENABLED=0` and `CUA_DRIVER_RS_UPDATE_CHECK=0`, and `~/.cua-driver/config.json` says false for both, for the desktop user and root.
- Under KasmVNC there is no `/dev/uinput`, so desktop-scope pointer input takes the driver's XTest route and keys that need focus take `"delivery_mode": "foreground"`; text into a terminal goes through the terminal's own route.

## What KasmVNC needs that is not obvious

- `-publicIP 127.0.0.1`: without it KasmVNC 1.4 asks public STUN servers for the machine's address before it opens any port, and exits silently when none answers, so a guest with no egress never gets a desktop.
- A WebSocket upgrade with no `Origin` header is closed; browsers always send one.
- Its pointer event is 11 bytes (a 16-bit button mask and two scroll deltas), not RFB's 6, so only KasmVNC's own web client, or a client written for it, can send input.
- `-httpd /usr/share/kasmvnc/www` makes 6080 serve the web client as well, so a browser pointed at the tunnelled port is a complete viewer.

## Not here yet

- A bridge serving the platform's `/desktop/*` HTTP contract on top of the driver (`desktopd`, milestone M2).
- A browser on this desktop owned by the platform's browser daemon (milestone M3).
- A persistent disk, a lifetime policy, billing and prewarmed desktops (milestone M4).
