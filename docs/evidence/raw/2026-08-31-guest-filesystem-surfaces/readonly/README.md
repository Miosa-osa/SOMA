# Which guest locations are read-only, and what a write into each answers - eval-1, 2026-09-01

The first failure run wrote to `/proc/version` and got the catch-all `failed` rather than
`denied`, which left the `denied` mapping unproved through a public surface. This probe asks the
guest what it actually mounts, then writes one byte into five locations and records the typed
cause each produces.

`mounts.txt` is `/proc/mounts` read out of the guest by a command through the same service. Every
mount in it is `rw`, including the root, which is an overlay with a writable upper over an EROFS
lower. There is therefore no read-only mount in this guest and `EROFS` is not reachable from a
guest path at all.

`NN-write-<path>.response` is the answer to a write into that path:

  /proc/version               failed        a procfs file with no write handler
  /sys/kernel/vmcoreinfo      denied        a sysfs node the guest refuses to open for writing
  /proc/sys/kernel/hostname   wrote 1 byte  procfs is mounted rw and this node is writable
  /etc/hostname               wrote 1 byte  ordinary path on the writable overlay
  /workspace/plain.txt        not_found     this sandbox has no /workspace directory

The second line is the one that proves the `denied` mapping live: the guest maps EACCES, EPERM
and EROFS to `denied`, and this write reaches one of the first two.
