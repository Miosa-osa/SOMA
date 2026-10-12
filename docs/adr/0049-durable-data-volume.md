# ADR 0049: A durable data volume that outlives its Instance

- Status: Proposed
- Date: 2026-10-12
- Extends: ADR 0002, ADR 0031, ADR 0036 (the "durable storage attaches as a separate volume or not at all" section)

## Context

Customers expect a persistent sandbox to keep its disk across stop and start, `soma-api` restarts and host reboots, as Firecracker sandboxes do.
Today a SOMA Instance keeps nothing: its writable head is an unlinked reflink of the Generation's overlay template, destroyed with the Instance (ADR 0002, ADR 0036), and the Instance registry lives on a 2 GiB tmpfs under `/var/lib/soma/state`, so the control plane reports `persistence: false` and refuses `persistent: true` on SOMA.

ADR 0036 already fixes the shape of any durable storage: a separately named volume with its own device, attach and detach transactions, quota and deletion, owned by the Host Runtime and excluded from every snapshot.
ADR 0031 (accepted, partly implemented) makes `soma-hostd` the owner of every managed Instance and of anything that must outlive the launching process.

Two facts constrain the design.
A snapshot restore brings back the guest's memory, including its cached view of the root overlay, so the root overlay cannot be swapped for a different disk after restore: the guest would believe the old, empty file system is on it (the same class of defect as the Firecracker ext4 warm-slot bug found on 2026-10-12).
A block device that exists in the Generation but was never mounted when the snapshot was captured has no cached state, so it can be bound to any backing file at restore.

## Decision

### Step A: a data volume

1. A new machine-contract revision adds one more virtio-blk device to the Generation, `vdc`, declared in the manifest and present in the captured device state, backed at capture time by a zero-length placeholder.
   Every sized Generation is recaptured once (per CPU class and size) when this ships.
2. At launch of a persistent Instance, `soma-hostd` binds `vdc` to the Instance's volume file instead of the placeholder.
   The guest agent mounts it at launch (after repair, with the launch page) and bind-mounts the persistent paths from it: `/home/sandbox`, `/workspace`, `/var/lib/docker` (so images and containers survive), and `/opt/user`.
   Nothing else is persistent in step A; the root file system stays ephemeral and keeps the ADR 0002 guarantee.
3. A volume is a named sparse ext4 file under a durable directory on the host (`SOMA_VOLUME_DIR`, default `/srv/soma/volumes/<volume-id>`), created by reflink from the Generation's existing overlay template of the same size class, so creating one costs the same as a head today.
   A sidecar `volume.json` next to it records volume id, owner Instance id, tenant, size class, Generation id and the state `attached | detached | deleting`.
4. Lifecycle, all hostd transactions with write-ahead records so a crash at any point recovers on start:
   - `create` (on persistent Launch), `attach` (Launch or Start), `detach` (guest `sync` and unmount, then the VM is destroyed, then the file is closed), `delete` (explicit, with the sandbox).
   - Stop = clean detach and destroy the VM; Start = restore the sterile snapshot (fast, no cold boot) and attach the same volume.
   - `soma-api` restart: volume files are untouched; running Instances are re-adopted exactly as today.
   - Host reboot: all VMs die; the volume files remain; the registry on tmpfs is gone, so `soma-hostd` rebuilds the set of stopped persistent Instances from the `volume.json` sidecars on start. The control plane marks previously running Instances stopped and the customer starts them.
5. The state directory stays on tmpfs in step A. It holds only running-VM records, and a running VM cannot survive a reboot, so persisting it adds cost without adding durability. Stopped Instances are described by their sidecar, not by the tmpfs.
6. A persistent Instance is pinned to its host: the volume is local. Loss of the host is loss of the data until the replication or backup work (not in this ADR). The control plane must show this in the capability text.
7. Encryption at rest: the volume directory is on the host's existing encrypted data volume where one exists (the `miosa-enc` LUKS mount on the Ryzen hosts); host-11 needs the same mount before it hosts persistent sandboxes.

### Step B: the whole disk (separate follow-up ADR)

The owner target is that the entire disk persists. Two routes, to be decided with measurements:

- B1: a persistent root. The Instance's root overlay head becomes a kept named file; Start is a cold boot of the Generation's kernel and initramfs with that head (no snapshot restore, roughly fifteen times slower than restore). Simple and obviously correct; start is about a second instead of milliseconds.
- B2: a second overlay layered at launch. After restore the agent mounts an overlayfs whose lower is the current root and whose upper is on `vdc`, and runs every command inside that merged root. Start stays a restore; system changes (apt installs, `/etc`, `/usr`) persist in the upper on the volume. Cost: a mount namespace change for every process the agent starts, and interaction with Docker's data root and with `/proc`.

Step A's device and lifecycle work is a prerequisite of both.

## Consequences

- The `filesystem.persist` capability becomes true for SOMA only for the data paths in step A, and is declared with that scope in the Generation manifest, so the control plane can say exactly what is kept.
- The ADR 0002 guarantee is kept: the overlay still holds no tenant state across Instances; the volume is a distinct, named, owned object.
- All Generations are recaptured once for the extra device; the preflight in `scripts/check-candidate-devices.sh` is updated for the new device list.
- Snapshots never include the volume (the device holds the placeholder at capture), so a restored Generation cannot carry another tenant's data.

## Open points (to settle in review)

- Quota and the enforcement point for volume size classes (reuse the 10/20/40 GiB classes).
- Whether `vdc` is also offered, empty, to non-persistent Instances for scratch use (no: keep the default device set small).
- Backup and cross-host replication for persistent volumes (follow-up).
- Billing metering of volume bytes for stopped Instances (control plane).
