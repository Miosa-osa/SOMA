# Local sandbox reality

This document explains exactly what SOMA creates on the development Mac.

## The short answer

SOMA does not itself turn a normal host process into a sandbox.
SOMA validates the request, selects a backend, passes the image and resource parameters to that backend, records evidence, and performs cleanup.
The selected backend creates the isolation boundary.

There are two working local paths on this Mac.

## Apple backend

The Apple path is a real Linux virtual machine.
SOMA invokes the pinned Apple Container runtime at `~/Library/Application Support/SOMA/apple-container/1.3.0/bin/container`.
That runtime uses Apple's Virtualization framework to create and manage a Linux VM.

The flow is:

```text
SOMA request
  -> image resolution and OCI identity
  -> Apple Container create
  -> Apple Virtualization Linux VM
  -> guest keeper process
  -> SOMA command execution
  -> Apple Container delete
```

The requested vCPU count, memory, image, network policy, command, timeout, and output limit are passed through SOMA's validated request model.
The VM is therefore configured using SOMA parameters, but the VM itself is created by the Apple runtime and Virtualization framework.

This is not a Docker container.
The receipt reports `macos_virtualization` and `hardware_virtual_machine` because the backend observed a VM lifecycle.

The guest image is still an OCI image.
The Apple runtime converts or prepares that image for its Linux VM implementation.
An OCI image does not imply a container; it can be used as the source for either a container rootfs or a VM disk.

## Docker backend

The Docker path creates a Linux container through Docker Desktop.
The flow is:

```text
SOMA request
  -> docker pull and image inspection
  -> docker create with limits and policy
  -> docker start
  -> docker exec
  -> docker rm --force
```

The container uses Docker Desktop's Linux VM kernel.
It is not a per-sandbox hardware VM.
The backend applies read-only root storage, dropped capabilities, no-new-privileges, a PID limit, temporary `/tmp`, network policy, bounded execution, and cleanup.

This path exists for local development because Docker Desktop is installed and healthy on the Mac.

## What was tested locally

The Apple path created and destroyed an Ubuntu 22.04 Linux VM and ran `/bin/true` inside it.
The first run took approximately 6.1 seconds including image preparation.
The cached run took approximately 1.78 seconds end to end.

The Docker path created and destroyed Ubuntu and Node 22 Linux containers.
Five cached Node 22 runs took approximately 1.19 to 1.24 seconds end to end.

The Mac has no `/dev/kvm` device.
Therefore the Linux KVM backend cannot execute on this host.
The Mac does have Apple hardware virtualization support, which is why the Apple VM backend works.

## What SOMA customizes and what it does not

SOMA customizes and enforces the portable contract:

- image reference and observed digest
- vCPU and memory request
- network policy
- direct command arguments
- timeout and output limits
- instance identity and lifecycle ownership
- cleanup and evidence

The current Mac implementation does not contain a custom hypervisor.
It delegates VM creation to Apple Container or container creation to Docker, depending on the selected backend.

The Linux custom Rust VMM is a separate backend under construction.
Its implementation must own KVM VM creation, guest memory, kernel boot, devices, guest-agent communication, and lifecycle teardown.

## How this compares with competitors

Competitor products usually separate the customer API from a Linux host fleet.
The customer's Mac sends a request to a remote Linux worker, and the worker creates a container or VM there.
The customer's Mac is normally an API client, not the sandbox host.

Products offering local Mac execution generally use one of two approaches:

1. A Linux VM managed by Apple Virtualization, Lima, Podman Machine, or Docker Desktop.
2. A process/container boundary that shares a Linux VM kernel.

They cannot use Linux KVM directly on macOS because Linux KVM is a Linux kernel interface exposed through `/dev/kvm`.

## Linux handoff

On the Linux machine, first run:

```sh
test -r /dev/kvm && test -w /dev/kvm
cargo run --locked -p soma-cli -- --backend kvm doctor
```

Passing those checks proves host access only.
The custom VMM still needs real guest memory, kernel boot, device setup, guest-agent readiness, and command execution before it can replace the Docker or Apple development adapters.

## Mac compatibility matrix

| Sandbox type | Works on macOS | Isolation boundary | Notes |
| --- | --- | --- | --- |
| SOMA Apple backend | Yes | Linux VM through Apple Virtualization | The primary non-Docker local path |
| SOMA Docker backend | Yes, with Docker Desktop | Linux container inside Docker's Linux VM | Shared Linux kernel |
| Native macOS process | Yes | No SOMA sandbox boundary | Never use this as a substitute for a sandbox |
| Linux namespaces directly | No, not on Darwin | Linux container | Requires Linux namespaces and cgroups |
| SOMA custom Rust KVM VMM | No, not natively | Linux KVM VM | Requires Linux `/dev/kvm` |
| Competitor Linux KVM VMM | No, not natively | Linux KVM VM | Requires Linux `/dev/kvm` |

## Running the custom VMM in Docker on a Mac

A Docker image can package and compile the custom VMM on macOS.
It can also run control-plane code that does not touch KVM.

That Docker image does not provide KVM by itself.
Docker Desktop already runs containers inside a Linux utility VM, and the nested Linux guest normally cannot access a usable `/dev/kvm` device for another VM layer.
Even if a development setup exposes nested virtualization, its behavior and timings are not equivalent to a bare-metal Linux KVM host.

Therefore a Dockerized custom VMM on this Mac is useful for builds and protocol tests, but not for creating or benchmarking the custom KVM sandbox.
