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
| `/etc/soma/responder.key` | `0400` | `0:0` | Exactly 32 raw bytes of the Generation-scoped X25519 responder private key. |
| `/dev` | `0755` | `0:0` | Empty directory; devtmpfs is mounted here by the agent. |
| `/proc`, `/sys`, `/mnt` | `0755` | `0:0` | Empty directories used as mount points. |

No shell, library, or other executable is required.
`/dev/console` is provided by the kernel-created initramfs node and by devtmpfs.

## Early-init behaviour the writer relies on

1. The agent mounts devtmpfs, procfs, and sysfs with fixed options.
2. It waits, within a ten-second budget, until `/sys/block` lists exactly `vda` and `vdb`.
3. It verifies the EROFS superblock magic on `/dev/vda` and mounts it read-only at `/mnt/lower`.
4. It verifies the ext4 magic and clean, error-free state on `/dev/vdb`, mounts it at `/mnt/upper`,
   requires the head to contain nothing but `lost+found`, and creates `upper/` and `work/`.
5. It mounts OverlayFS at `/mnt/root`.
6. It reads `/etc/soma/responder.key`, overwrites the file with zeroes, and unlinks it.
7. It moves `/dev`, `/proc`, and `/sys` into the composed root, moves the composed root over
   `/`, and enters it with `chroot` as `switch_root` does, because `pivot_root` cannot leave the
   initial ramfs.
8. It waits at the disconnected repair point for the launch page.

The Generation snapshot is captured while the agent waits in step 8.

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
