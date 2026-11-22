# SOMA desktop: Xorg, Openbox, the cua driver and an on-demand viewer

## Purpose

This directory turns a Linux desktop image into a SOMA Generation an agent can drive and a person can watch.
A desktop is not a new kind of machine: it is an ordinary Generation (the same guest agent, devices and tunnel as every sandbox) at a desktop shape, on the desktop kernel variant, whose session is started after the machine is Ready.

The design keeps three jobs apart:

| Job | Piece | Runs |
| --- | --- | --- |
| The screen | Xorg with the dummy video driver, a 1920x1080 frame buffer in memory, no GPU, no VNC | always |
| The agent's hands and eyes | `cua-driver` 0.55.0 (MIT) as `cua-driver serve` on its owner-only Unix socket, reading the AT-SPI tree and injecting through XTEST | always |
| A person watching and driving | Sunshine (captures the screen, software H.264) and moonlight-web (the browser client, relayed over WebSockets) on guest port 6080, reached through `GET /v1/sandboxes/{id}/tunnel/6080` | only while someone watches |

The window manager is Openbox and the one application is `xfce4-terminal`.
Because the viewer is separate from the screen, the agent never depends on it, and a desktop nobody watches pays nothing for encoding.

## Pinned inputs

| Input | Pin |
| --- | --- |
| Ubuntu | `ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517`; Xorg, Openbox, udev and the terminal from its archive |
| cua-driver | `cua-driver-rs-0.55.0-linux-x86_64.tar.gz`, SHA-256 `fd863defadb13573d53fd327e7d09268460a658224651329cb0b857b0c8752ac` |
| Sunshine | `sunshine_2026.914.233613-1+ubuntu24.04_amd64.deb`, SHA-256 `c38e9c705f650f8705f61702717e99bec34044c5028fcb8f23a08fe041292c21` |
| moonlight-web | `moonlight-web-x86_64-unknown-linux-gnu.tar.gz` v3.0.2, SHA-256 `4b6de5c1c35553e9d35bd67a3343ebd01f67d0a4216116c09b78b12adccc03df` |
| Kernel | the desktop variant, `kernel/build-variant.sh desktop`: version 1 plus evdev and uinput only |

Only the MIT cua driver binary, its licence and its third-party notices are installed; the build fails if any `cua-spacesd`, `cua-relay` or `cua-perception` file appears.

## Build

```sh
kernel/build-variant.sh desktop
docker build -f workloads/desktop/base.Dockerfile -t soma-desktop-base:3 workloads/desktop
docker build --build-arg BASE_IMAGE=soma-desktop-base:3 -t soma-desktop:3 workloads/desktop
scripts/desktop-generation.sh soma-desktop:3 soma-desktop:small-3 <store> <fs-tools> small
```

The stage build pairs moonlight-web with Sunshine once (`viewer/pair.py`): it starts the screen, both programs and asks Sunshine's own API to accept moonlight-web's PIN, then makes moonlight-web's only user its default user so the page opens without a login.
Who may open the page is decided at the tunnel, by the platform.

Windows programs run through Wine in an optional layer, so a plain desktop does not carry it:

```sh
docker build -f workloads/desktop/wine.Dockerfile --build-arg DESKTOP_IMAGE=soma-desktop:3 -t soma-desktop-wine:3 workloads/desktop
```

The guest kernel has no binfmt_misc, so a program starts as `wine program.exe`.

## How the session runs

The snapshot holds no running process, so the desktop starts after Ready:

1. `soma-desktop-session start` runs as root through exec.
   It moves itself into the cgroup `/soma-desktop`, names that cgroup in `/run/soma/survivors` so the guest agent spares it, and starts udev (there is no init system to start it).
2. It starts `session.sh` as the desktop user, which brings up, each waiting for the one before: the session D-Bus, Xorg on `:0`, the accessibility bus, Openbox, and `cua-driver serve`.
3. It waits for `session_ready` and prints every milestone as JSON.

`soma-desktop-session viewer start` starts Sunshine and moonlight-web on 6080 and waits for `viewer_ready`; `viewer stop` ends them.
Sunshine creates its keyboard and pointer through `/dev/uinput`; udev announces them and Xorg adds them through libinput.

`soma-desktop-call` runs one driver command as the desktop user, for example `soma-desktop-call call get_desktop_state '{"max_image_dimension":0}'`.

## cua-driver configuration

- `standard` permission mode with `session/cua-capabilities.yaml` as a ceiling: the desktop display, the terminal's and Openbox's windows, and one writable directory for screenshots.
  Recording, scripting, browser control and process termination are not in it.
- Telemetry and the update check are off for every caller, in the environment and in the persisted preference.
- An application the agent launches is not given keyboard focus; `bring_to_front` gives it, and a person's click does the same.

## Not here yet

- Pause and resume of a running desktop with its memory (ADR 0034).
- A bridge serving the platform's `/desktop/*` HTTP contract on top of the driver.
- Audio: the guest has no sound device, so the stream has none.
