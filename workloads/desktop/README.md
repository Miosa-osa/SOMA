# SOMA Linux desktop

The Ubuntu 24.04 desktop runs in a SOMA virtual machine using the desktop kernel variant.
Xorg provides a virtual display, Openbox manages windows, and Xfce Terminal is the default terminal.
The CUA driver reads the display, inspects accessibility through AT-SPI, and sends input to Xorg.

The browser viewer shares the same display.
Sunshine captures and encodes it as H.264, and moonlight-web sends the stream and human input over WebSockets on guest port 6080.
The CUA driver can operate while these viewer processes are stopped.
The platform must authenticate and authorize access before opening the viewer tunnel.
The image build pairs Sunshine and moonlight-web, making the paired host available when the authorized viewer opens.

## Build the image and Generation

Run the following from the SOMA repository root.
The image pins the Ubuntu base, CUA driver, Sunshine, and moonlight-web in the Dockerfiles.

```sh
docker build -f workloads/desktop/base.Dockerfile -t soma-desktop-base:2 workloads/desktop
docker build --build-arg BASE_IMAGE=soma-desktop-base:2 -t soma-desktop:2 workloads/desktop
```

An existing root filesystem can supply the base if it includes Xorg with the dummy video driver, Openbox, Xfce Terminal, D-Bus, and the configuration at `/etc/soma-desktop/xorg.conf`.

```sh
workloads/desktop/import-rootfs.sh <rootfs.ext4> desktop-rootfs:1
docker build --build-arg BASE_IMAGE=desktop-rootfs:1 -t soma-desktop:2 workloads/desktop
```

Compile and capture the Generation on a KVM host after building SOMA and its filesystem tools.

```sh
kernel/build-variant.sh desktop
cargo build --release -p soma-generation --example prepare_generation -p soma-local --example capture_snapshot
scripts/desktop-generation.sh soma-desktop:2 soma-desktop:small-2 <store> <fs-tools> small
```

The `small` shape has 2 vCPUs, 4096 MiB of memory, and 20480 MiB of writable storage.
The desktop kernel variant enables evdev and `uinput` for the browser viewer.
The kernel verifier reports the explicit input override and checks that physical input drivers stay disabled.

Optional Windows application support uses a separate Wine layer.

```sh
docker build -f workloads/desktop/wine.Dockerfile \
  --build-arg DESKTOP_IMAGE=soma-desktop:2 -t soma-desktop-wine:2 workloads/desktop
```

Start applications with `wine program.exe`.
The layer waits for wineserver to write the registry before publishing the Wine prefix.

## MIOSA desktop API image

Build `desktopd` from MIOSA Compute's `envd` module at the revision selected for deployment.

```sh
cd /path/to/miosa-compute/envd
GOOS=linux GOARCH=amd64 CGO_ENABLED=0 go build -trimpath -o /path/to/bridge/desktopd ./cmd/desktopd
```

From the SOMA repository root, add the executable to the desktop image and use the resulting image as the source for Generation capture.

```sh
docker build -f workloads/desktop/platform.Dockerfile \
  --build-arg DESKTOP_IMAGE=soma-desktop:2 -t soma-desktop:platform-2 /path/to/bridge
```

Before starting the session, the platform writes that computer's bearer credential to `/run/desktopd/token` as a regular file owned by `ubuntu` with mode 0600.
The credential is provisioned per computer and stays out of the image.
On this image, session startup waits for `desktopd` on guest port 49985.
It serves the same authenticated desktop HTTP routes as envd and uses the CUA driver socket on the guest.
The platform opens the desktop port only through its authenticated tunnel.

## Start and operate a desktop

The certified startup snapshot starts the guest agent.
Once the VM reports Ready, run `soma-desktop-session start` as root through guest exec.
The launcher starts the session in the `/soma-desktop` cgroup so the guest agent can preserve it when the startup command exits.
The command returns the session state and timestamps for D-Bus, Xorg, accessibility, Openbox, and driver startup.

Run `soma-desktop-call doctor` to check the driver.
For example, `soma-desktop-call call get_desktop_state '{"max_image_dimension":0}'` reads the display and current windows.
The capability manifest in `session/cua-capabilities.yaml` limits accessible applications, operations, and screenshot storage.
The driver wrapper disables telemetry and automatic update checks.

Run `soma-desktop-session viewer start` when a human opens the desktop.
Tunnel guest port 6080 using the authorized SOMA endpoint.
Run `soma-desktop-session viewer stop` when the viewer closes.
The stop command waits for the viewer supervisor to terminate and reap Sunshine and moonlight-web.
The Xorg session and CUA driver remain available.

The [original Xfce and KasmVNC experiment](../../docs/evidence/2026-11-20-desktop-generation-cua-m1.md) records the earlier design and measurements.
