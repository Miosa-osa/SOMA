# A sandbox that declares no writable disk gets none - 2026-08-31

## Status: current

The five-slot device surface used to be fixed. Every machine was built with an immutable root, a
private writable overlay, a network device, vsock, and entropy, whether or not it had any use for
the last two. The device set now follows the Generation's declaration, and this is the first live
evidence of what that costs and what it saves.

The saving is the private head clone. `admitted` to `machine_launched` is the segment where a
launch opens the prepared artifacts and gives the Instance its own copy-on-write disk head. A
Generation that declared no writable storage has no head to clone, and the segment collapses from
a median of 5.79 ms to 0.36 ms at concurrency 1, and from 94.67 ms to 0.23 ms at concurrency 100.

## Run identity

| | |
| --- | --- |
| Commit | `2423b9493f25b4b8a6fcdf8afaae48fe21b61ba5` |
| Host | eval-1, Linux 6.8.0-138-generic, 80 threads, Intel Xeon Gold 6138 @ 2.00 GHz |
| Head directory | `/srv/soma/heads`, XFS with reflink, so the writable arm takes the fast clone path |
| Build | release |
| Image | `busybox:stable-musl`, `linux/amd64` |
| Command | `soma --format json --backend kvm run busybox:stable-musl -- /bin/echo soma-ok` |
| Retained | [`raw/2026-08-31-declared-device-set/`](raw/2026-08-31-declared-device-set/) |

Two Generations were compiled from the same image at the same memory size. They differ in one
declared field, so they differ in exactly one device.

| Generation | Memory | Writable storage | Devices built |
| --- | ---: | ---: | --- |
| `ds-rw-128-1024` | 128 MiB | 1024 MiB | root, overlay, vsock, entropy |
| `ds-ro-128-0` | 128 MiB | 0 | root, vsock, entropy |

Neither has a network device: both declare the fail-closed isolated policy, which is now a machine
with no network device rather than one whose link is held down. Both were captured once and every
run below is a restore of that capture.

## What each machine is

The read-only Generation's kernel command line, read from inside its own guest:

```text
console=ttyS0 reboot=k panic=1 nomodule random.trust_cpu=off pci=off acpi=off noapic
cryptomgr.notests virtio_mmio.device=4K@0xd0000000:5:0 virtio_mmio.device=4K@0xd0003000:8:3
virtio_mmio.device=4K@0xd0004000:9:4 rdinit=/init soma.lower=/dev/vda
```

Three declarations, not five. The overlay page and the network page keep their addresses and are
simply never declared, so the root is still the first block device and nothing else moved. The
writable Generation's line carries four declarations and `soma.upper=/dev/vdb` as well.

What each guest reports about itself:

| Observation | `ds-rw-128-1024` | `ds-ro-128-0` |
| --- | --- | --- |
| `ls /sys/bus/virtio/devices` | `virtio0 virtio1 virtio2 virtio3` | `virtio0 virtio1 virtio2` |
| `ls /sys/block` | `vda vdb` | `vda` |
| `ls /sys/class/net` | `lo` | `lo` |
| `touch /writable-ok` | succeeds | `Read-only file system` |
| `echo scratch > /tmp/x` | succeeds | succeeds |
| `mount \| grep -c overlay` | 1 | 0 |

The read-only guest still has a writable `/tmp` and `/run`, because those are per-Instance tmpfs
mounts rather than anything on a disk. Its snapshot directory holds `memory.raw` and
`state.somasnap` and no `overlay.raw`: there is no sterile template, because there is nothing to
be a template of.

## The private head clone

`admitted` to `machine_launched`, in milliseconds. Concurrency 1 is 30 sequential sandboxes;
concurrency 100 is one cohort released together by a barrier, with receipts parsed only after the
last sandbox exits.

| Configuration | Concurrency | min | p50 | p95 | max |
| --- | ---: | ---: | ---: | ---: | ---: |
| writable | 1 | 3.10 | 5.79 | 21.68 | 22.15 |
| read-only | 1 | 0.35 | **0.36** | 0.39 | 0.39 |
| writable | 100 | 84.27 | 94.67 | 100.10 | 103.07 |
| read-only | 100 | 0.15 | **0.23** | 0.27 | 0.42 |

Every one of the 260 sandboxes reached its command and produced the expected output.

Two things are worth separating. The median saving at concurrency 1 is about 5.4 ms, which is
real but small next to a 33 ms time to first command. The saving at concurrency 100 is 94 ms,
which is most of the difference between the two totals, and it is also where the variance was:
the writable segment spread from 84 to 103 ms under contention, and the read-only segment stayed
inside 0.15 to 0.42 ms. Removing the clone removes the spread as well as the cost, because there
is no longer a filesystem operation on the request path to contend for.

## What that does to the whole launch

Time to first command, in milliseconds:

| Configuration | Concurrency | p50 | p95 |
| --- | ---: | ---: | ---: |
| writable | 1 | 32.78 | 49.18 |
| read-only | 1 | 26.19 | 28.75 |
| writable | 100 | 139.6 | 151.7 |
| read-only | 100 | **30.8** | 43.3 |

At concurrency 100 a read-only sandbox reaches its first command in less time than a writable one
takes to acquire its disk. The rest of the launch is almost unchanged: `machine_launched` to
`ready`, which is the restore, the authenticated session, and the guest's own repair, runs
25.49 ms at the writable median and 24.26 ms at the read-only one. The guest saves about a
millisecond by skipping the ext4 superblock verification, the upper mount, the work directory,
and the `OverlayFS` composition, which is roughly what those cost and no more. The launch-path
saving is the clone, not the boot.

## What the read-only guest skips

- It waits for one virtio block device instead of two, so a device that never appears is a
  timeout rather than a wait that could never end.
- It verifies the EROFS superblock and mounts the root read-only, and then switches into it. It
  performs no ext4 superblock verification, no upper mount, no `upper` or `work` directory
  creation, and no `OverlayFS` composition. The sterile-head checks are untouched and still run
  in full for every machine that has a head.
- It leaves `/etc/hostname` and `/etc/machine-id` alone, because it has nowhere to put them. The
  kernel hostname, which is what `gethostname` reads, is replaced exactly as before.
- It installs no interface identity, address, netmask, route, resolver, or hosts file, because
  there is no interface, and raises loopback so that a workload talking to its own address still
  works.

## What the overlay turned out to be load-bearing for

The first read-only boot failed at `MoveMounts` with `EROFS`, and the second failed in identity
repair for the same reason. The immutable root busybox publishes carries `/dev` but neither
`/proc` nor `/sys` nor `/run`: a container runtime creates those at start, and an OCI image is
under no obligation to ship them. That had never mattered, because the writable overlay made the
missing directories creatable at boot. So the overlay was quietly load-bearing for the
pseudo-filesystem mount points, not only for tenant writes.

The fix is in the compiler rather than the guest: normalization now adds `/dev`, `/proc`, `/run`,
`/sys`, and `/tmp` to every canonical tree that does not already carry them, after every layer is
applied, and leaves an entry the image did publish exactly as it published it. Every Generation
identity moves, which this change was already doing. A machine with an overlay is unaffected,
because it was creating those directories at boot anyway.

## What this is not

- It is not a claim about the network device. Both arms declare the isolated policy, so both were
  built without one, and nothing here measures a machine that has one.
- It is not a certification result. Both Generations are Candidates launched with
  `SOMA_ALLOW_UNCERTIFIED_GENERATION=1`.
- It is not a claim about anything larger than busybox. A read-only `node:22` would restore a
  1 GiB memory image, and the clone it skips would be the same size it is here.
