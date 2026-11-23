# Local Docker Node 22 sandbox validation - 2026-08-29

This record covers the first working SOMA sandbox backend on the development Mac.

## Environment

- Host: Apple Silicon macOS.
- Runtime: Docker Desktop, Linux ARM64 engine.
- Image: `node:22`.
- Observed image digest: `sha256:8a34c4ab3ea2c5cd194f07e317b2a8f09461d3c8b05c4e34c8ccd56d56024c4d`.
- Shape: 1 vCPU and 1024 MiB memory.
- Network: disabled.

## Workload

SOMA created a container, executed `/usr/local/bin/node --version`, returned `v22.23.2`, and removed the container.
Five consecutive one-shot runs completed successfully.
Each receipt reported `backend: docker_container`, `isolation: linux_container`, and complete cleanup evidence.

## Timing

The end-to-end receipt elapsed times were approximately 1.19 s, 1.19 s, 1.24 s, 1.19 s, and 1.19 s from acceptance through cleanup.
The launch milestone occurred approximately 1.01 s after acceptance because the current path invokes the Docker CLI and resolves the image on demand.
These numbers are local development measurements, not a claim of millisecond VM startup and not comparable to a warmed custom VMM.

## Boundary

This backend is a constrained Linux container inside Docker Desktop's Linux VM.
It is not a per-sandbox hardware-isolated virtual machine.
The production Linux custom Rust VMM remains a separate implementation task.
