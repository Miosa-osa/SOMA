# SOMA runtime footprint measurements

## Linux x86_64 release artifacts

The alpha.4 numbers come from two Linux x86_64 draft release archives whose SHA-256 digests matched the release checksum file.
Compressed sizes are the archive lengths in bytes.
Executable sizes are the corresponding tar entry lengths before compression.

| Draft alpha.4 artifact | Exact bytes | Measurement |
| --- | ---: | --- |
| Linux client archive | 5,461,963 | Compressed CLI and MCP package |
| Linux host archive | 7,704,292 | Compressed API, VMM, network broker, host daemon, and guest agent package |
| `soma` | 6,539,576 | Uncompressed CLI binary |
| `soma-mcp` | 10,372,248 | Uncompressed MCP binary |
| `soma-api` | 16,390,424 | Uncompressed API binary |
| `soma-vmm` | 1,932,976 | Uncompressed VMM binary |
| `soma-guest-agent` | 905,392 | Uncompressed guest agent binary |
| `soma-hostd` | 1,299,472 | Uncompressed host daemon binary |
| `soma-netd` | 1,383,728 | Uncompressed network broker binary |

The client archive digest is `2205c0c3b220b40e40033cc139c5a93bb977c1743685551a40662df5e8782c9c`.
The host archive digest is `d0ea687af08701db9c4b061cec82167a5b1c617e13c2ad88cf9c792f952160fa`.
The [published alpha.3 release](https://github.com/Miosa-osa/SOMA/releases/tag/v1.0.0-alpha.3) lists the comparable Linux client archive at 5,334,232 bytes and Linux host archive at 7,521,930 bytes.

Archive sizes exclude the guest kernel, OCI workload image, captured memory, and writable disk.
Executable sizes describe bytes on disk.

## Earlier desktop process sample

An x86_64 KVM rehearsal used runtime `5ab6f014108b` and the lightweight Xorg/Openbox desktop with a corrected guest session script.
After agent input completed, the run sampled individual guest processes using `ps -C <name> -o rss=,comm=`.
The units below are KiB.

```text
77340 Xorg
21896 openbox
76128 cua-driver
```

Xorg and Openbox summed to 99,236 KiB, or 96.9 MiB of process RSS.
The computer-use driver reported 76,128 KiB, or 74.3 MiB of process RSS.
A sum of process RSS can count shared pages more than once.
The sample excludes total guest memory, VMM overhead, and the current Xvfb/voxl viewer.

The [earlier KVM boot proof](2026-08-29-x86_64-pvh-kernel-boot.md#host-side-resident-overhead-for-one-1-vcpu-guest-with-256-mib-ram) reports about 3.6 MiB of non-guest resident memory in one debug cold-boot sample.
That measurement applies only to its 1-vCPU fixture and excludes the guest RAM and kernel image buffer.
A current engine comparison needs a matched workload and shape, with the same measurement boundary and cache state.
