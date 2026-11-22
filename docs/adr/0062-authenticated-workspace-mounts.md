# 0062. Authenticate workspace mount transitions

The guest mounts an assigned, persistent virtio block device before it reports readiness.
A live disk handoff also needs a filesystem boundary before the VMM can release that backing.

Guest API level 6 adds an authenticated, operation-bound request to mount or unmount the declared workspace.
The host probes this level before sending the request.
The request has no guest-supplied paths, disk IDs, or shell arguments.
Guests with no declared workspace reject the request.

The agent removes bind mounts before it unmounts ext4 and restores the binds if the kernel refuses to unmount.
It acquires an exclusive block-device claim before reporting success, so mounts held in another mount namespace prevent handoff.
An incomplete rollback is reported separately from a refused transition.
The host must only replace the paused block backing after a successful outcome.
The guest drops its exclusive claim only when mounting again.

This guest contract does not change the native volume API on its own.

The worker sequences mount exchanges and disk changes under its existing single-owner command loop.
It retains the declared disk slot and capacity and accepts only file descriptors opened and locked by the owner.
It never evicts a disk whose guest refused to unmount.
If rebinding a disk fails, the worker keeps the slot offline.
If the guest refuses to mount the newly bound disk, the worker releases the disk again.
An uncertain mount or failed rollback terminates the worker.
Checkpoint and suspension requests are refused while the workspace slot is offline.

The existing private machine-host socket accepts a workspace command for the exact Instance and its already-owned volume.
It resolves the attachment from hostd instead of accepting a path.
The supervisor validates the capacity, opens the file without following symlinks, and transfers its locked descriptor to the jailed worker.
The native volume API sends every running Instance's attach or detach to this private machine host.
Before detaching, the host validates the operation and owner, then asks the worker to unmount the filesystem and release the disk.
After the worker releases the disk, the volume store checks its lock and publishes the detached sidecar.
Before reattaching, the store verifies the original owner, tenant, Generation and disk class, then repairs the disk without publishing a new sidecar.
The store marks the disk attached only after the worker confirms the mount and holds the lock.
If publication fails, the caller gets a failure and can retry against the same running worker without repeating its mount or unmount.
A locked disk cannot be erased through the volume store.
