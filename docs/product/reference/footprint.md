# SOMA size and memory footprint

A SOMA download, an instance's memory, and its workload image are separate measurements.
See the [runtime comparison](runtime-comparison.md) for peer executable sizes and capabilities.
Use the one that matches what you are comparing.

## Download and binary sizes

These are Linux x86_64 measurements.
MB in this table is decimal, where 1 MB is 1,000,000 bytes.

| Component or package | Size | Meaning |
| --- | ---: | --- |
| Alpha.3 CLI and MCP package | 5.33 MB | Compressed download |
| Alpha.4 draft CLI and MCP package | 5.46 MB | Compressed archive |
| Alpha.3 host package | 7.52 MB | Compressed download |
| Alpha.4 draft host package | 7.70 MB | Compressed archive |
| Alpha.4 VMM | 1.93 MB | One uncompressed host executable |
| Alpha.4 guest agent | 0.91 MB | One uncompressed executable packaged inside the guest |
| Alpha.4 API service | 16.39 MB | One uncompressed host executable |

Guest kernels, workload images, snapshot files, and attached storage are sized separately.
See the [artifact measurements and checksums](../../engineering/evidence/2026-11-22-runtime-footprint.md#linux-x86_64-release-artifacts).

## Memory while a workload runs

The instance shape is the configured guest memory capacity.
Resident memory is the physical memory a process is using at a measurement point.
Report the guest, host VMM, and any shared pages with their measurement method when comparing engines.

One earlier Xorg/Openbox desktop sample measured about **97 MiB of combined process RSS** for the display server and window manager and **74 MiB of process RSS** for the computer-use driver.
The reference small desktop shape configures **4 GiB of guest RAM**.
Those process figures describe the earlier desktop, and shared pages can be counted in more than one process. [Desktop process sample](../../engineering/evidence/2026-11-22-runtime-footprint.md#earlier-desktop-process-sample)

Current desktop RAM, viewer CPU, and workload image size depend on the image, apps, and viewer activity.
Compare SOMA and Firecracker with the same guest image, instance shape, workload, host, and cache conditions before drawing a footprint conclusion.
