# Run a desktop on a SOMA instance

A desktop Generation packages a Linux graphical session for an ordinary SOMA instance.
It uses the desktop kernel variant and starts the desktop session after the instance is Ready.

| Role | Component | When it runs |
| --- | --- | --- |
| Display | Xvfb with a shared-memory framebuffer and Openbox | During the desktop session |
| Agent input and accessibility | `cua-driver` through its owner-only Unix socket, using XTEST and AT-SPI | During the desktop session |
| Browser viewer | voxl serves the display on guest port 6080 through the SOMA tunnel | The viewer listens during the session and streams when a browser connects |
| Optional streaming viewer | Sunshine and moonlight-web on guest port 6081 by default | Started separately with the viewer command |

The desktop includes PCManFM, a dock, wallpapers, Chrome, and a terminal.
The agent driver and browser viewer connect to the same display independently.
The integrating platform controls who may open the tunnel.

## Pinned inputs

| Input | Pin |
| --- | --- |
| Ubuntu | `ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517` |
| voxl | Source revision pinned in the [Dockerfile](Dockerfile) |
| cua-driver | `cua-driver-rs-0.55.0-linux-x86_64.tar.gz`, SHA-256 `fd863defadb13573d53fd327e7d09268460a658224651329cb0b857b0c8752ac` |
| Sunshine | `sunshine_2026.914.233613-1+ubuntu24.04_amd64.deb`, SHA-256 `c38e9c705f650f8705f61702717e99bec34044c5028fcb8f23a08fe041292c21` |
| moonlight-web | `moonlight-web-x86_64-unknown-linux-gnu.tar.gz` v3.0.2, SHA-256 `4b6de5c1c35553e9d35bd67a3343ebd01f67d0a4216116c09b78b12adccc03df` |
| Kernel | the desktop variant, `kernel/build-variant.sh desktop`: version 1 plus evdev and uinput only |

Only the MIT cua driver binary, its licence and its third-party notices are installed; the build fails if any `cua-spacesd`, `cua-relay` or `cua-perception` file appears.

## Build

Set `SOMA_STORE` to the Generation store directory and `SOMA_FS_TOOLS` to the directory containing the pinned filesystem tools.
Run the Generation build on a Linux KVM host prepared with the [host setup guide](../../docs/product/operations/server-setup.md).

```sh
kernel/build-variant.sh desktop
docker build -f workloads/desktop/base.Dockerfile -t soma-desktop-base:3 workloads/desktop
docker build --build-arg BASE_IMAGE=soma-desktop-base:3 -t soma-desktop:3 workloads/desktop
scripts/desktop-generation.sh soma-desktop:3 soma-desktop:small-3 "$SOMA_STORE" "$SOMA_FS_TOOLS" small
```

The build pairs moonlight-web with Sunshine once for the optional streaming viewer.
The pairing procedure lives in [the viewer build script](viewer/pair.py).

Windows programs run through Wine in an optional layer, so a plain desktop does not carry it:

```sh
docker build -f workloads/desktop/wine.Dockerfile --build-arg DESKTOP_IMAGE=soma-desktop:3 -t soma-desktop-wine:3 workloads/desktop
```

The guest kernel has no binfmt_misc, so a program starts as `wine program.exe`.

## How the session runs

The snapshot holds no running process, so the desktop starts after Ready:

1. `soma-desktop-session start` runs as root through exec.
   It registers the desktop cgroup so the guest agent keeps session processes alive between commands.
2. The launcher starts the desktop user's session, including D-Bus, Xvfb on `:0`, the accessibility bus, Openbox, voxl, and `cua-driver serve`.
3. It waits for `session_ready` and prints every milestone as JSON.

Reach voxl through `GET /v1/sandboxes/{instance}/tunnel/6080`.
`soma-desktop-session viewer start` starts the optional Sunshine and moonlight-web viewer, and `viewer stop` ends it.

`soma-desktop-call` runs one driver command as the desktop user, for example `soma-desktop-call call get_desktop_state '{"max_image_dimension":0}'`.

## cua-driver configuration

- `standard` permission mode with `session/cua-capabilities.yaml` as a ceiling: the desktop display, the terminal's and Openbox's windows, and one writable directory for screenshots.
  Recording, scripting, browser control and process termination are not in it.
- Telemetry and the update check are off for every caller, in the environment and in the persisted preference.
- An application the agent launches is not given keyboard focus; `bring_to_front` gives it, and a person's click does the same.

## Verification

The [desktop proof harness](../../scripts/desktop-proof.sh) checks launch, agent input, viewer access, and cleanup on KVM.
Verify suspend, resume, and viewer reconnect with the desktop Generation you intend to run.
The reference viewer serves video and input with audio disabled.
