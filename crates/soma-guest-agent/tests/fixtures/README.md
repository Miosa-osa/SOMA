# Guest agent initramfs placement

This directory documents how the statically linked `soma-guest-agent` binary is injected into a
Generation initramfs.
The Generation compiler owns the deterministic `newc` archive writer; this crate only produces
the binary and states the contract that writer must satisfy.

## Build the artifact

```sh
scripts/build-guest-agent.sh
```

The script builds `target/x86_64-unknown-linux-musl/release/soma-guest-agent` with
`-C target-feature=+crt-static`, refuses a dynamically linked result, and prints the byte size
and SHA-256 that the Generation manifest pins.

## Initramfs contents

| Archive path | Mode | Owner | Contents |
| --- | ---: | ---: | --- |
| `/init` | `0755` | `0:0` | The `soma-guest-agent` binary itself; the kernel executes it as PID 1. |
| `/bin/soma-guest-agent` | `0755` | `0:0` | The same binary under its own name. |
| `/dev` | `0755` | `0:0` | Empty directory; devtmpfs is mounted here by the agent. |
| `/dev/console`, `/dev/null` | `0600`, `0666` | `0:0` | Character nodes the Rust runtime needs before devtmpfs is mounted. |
| `/lower`, `/newroot`, `/overlay`, `/proc`, `/sys` | `0755` | `0:0` | Empty directories used as mount points. |

Layout version 3 carries no secret at all.
The archive holds exactly two byte bodies and both are executables.
Layout v2 carried a Generation-scoped responder private key at `/etc/soma/responder.key`; [ADR 0024, per-Instance guest responder authority](../../../../docs/adr/0024-per-instance-guest-responder-authority.md) removed it, and `verify_initramfs` rejects a v2 archive because that entry is not in the v3 allowlist.

No shell, library, or other executable is required.
`/dev/console` is provided by the kernel-created initramfs node and by devtmpfs.

## Early-init behaviour the writer relies on

1. The agent mounts devtmpfs, procfs, and sysfs with fixed options.
2. It waits, within a ten-second budget, until `/sys/block` lists exactly `vda` and `vdb`.
3. It verifies the EROFS superblock magic on `/dev/vda` and mounts it read-only at `/mnt/lower`.
4. It verifies the ext4 magic and clean, error-free state on `/dev/vdb`, mounts it at `/mnt/upper`,
   requires the head to contain nothing but `lost+found`, and creates `upper/` and `work/`.
5. It mounts OverlayFS at `/mnt/root`.
6. It moves `/dev`, `/proc`, and `/sys` into the composed root, moves the composed root over
   `/`, and enters it with `chroot` as `switch_root` does, because `pivot_root` cannot leave the
   initial ramfs.
7. It waits at the disconnected repair point for the launch page, which is where the fresh
   per-Instance responder static secret arrives.

The Generation snapshot is captured while the agent waits in step 7.

## Kernel configuration the agent requires

- `CONFIG_DEVMEM=y` so the launch page can be mapped at guest-physical `0xd0100000`.
  `CONFIG_STRICT_DEVMEM` may stay enabled because that address is not System RAM.
  `CONFIG_IO_STRICT_DEVMEM` must not be enabled unless no driver claims that page.
- `CONFIG_VSOCKETS=y`, `CONFIG_VIRTIO_VSOCKETS=y`, and the `/dev/vsock` character device.
- `CONFIG_HW_RANDOM=y` and `CONFIG_HW_RANDOM_VIRTIO=y` for `/dev/hwrng`.
- `CONFIG_EROFS_FS=y`, `CONFIG_EXT4_FS=y`, `CONFIG_OVERLAY_FS=y`, `CONFIG_DEVTMPFS=y`,
  `CONFIG_PROC_FS=y`, `CONFIG_SYSFS=y`, `CONFIG_TMPFS=y`.
- `CONFIG_VIRTIO_BLK=y`, `CONFIG_VIRTIO_NET=y`, `CONFIG_VIRTIO_MMIO=y`, `CONFIG_INET=y`.

## Readiness probe

The host readiness probe executes `/proc/self/exe --soma-ready-probe-v1` through the production
executor.
The binary exits with status zero and produces no output when invoked with that argument.
