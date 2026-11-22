# SOMA Linux desktop

The Ubuntu 24.04 desktop runs in a SOMA virtual machine using the desktop kernel variant.
Xorg provides a virtual display, Openbox manages windows, and Xfce Terminal is the default terminal.
The CUA driver reads the display, inspects accessibility through AT-SPI, and sends input to Xorg.

The browser viewer shares the same display.
Sunshine captures and encodes it as H.264, and moonlight-web sends the stream and human input over WebSockets on guest port 6080.
The CUA driver can operate while these viewer processes are stopped.
The platform must authenticate and authorize access before opening the viewer tunnel.

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
The desktop kernel enables input devices and `uinput` for the browser viewer.

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
