# 0070 - Checkpoint transfer on the host listener

Status: Accepted for implementation

## Context

ADR 0064 lets a host-local consumer stream each artifact of a published capture for an off-host backup, and ADR 0065 and ADR 0066 let it install a verified backup again.
Both are reachable only through the `soma` host command, which reads a receipt on standard input and moves one artifact per process.
The control plane drives every other checkpoint operation through the loopback soma-api listener, so a backup needed a second transport onto every host, and a capture could not outlive its host without one.

## Decision

The loopback host listener serves four checkpoint transfer routes, to the platform tenant only.
Any other tenant is answered `404 route_not_found`, as if the routes did not exist.
The public runner exposes none of them.

- `POST /v1/checkpoints/export` names the owner, the full capture receipt and one artifact (`state`, `memory`, `overlay` or `workspace`).
  It answers the artifact as a raw `application/octet-stream` body with its exact `content-length` and the receipt's digest in `soma-artifact-digest`.
  A read or digest failure part way closes the connection short of the announced length.
- `PUT /v1/checkpoints/imports/{import}/{artifact}` receives one artifact as a raw body with a `content-length`.
  The bytes go into a private stage in the checkpoint root on the encrypted mount, named `.staging-import-{import}`, and the answer carries their SHA-256.
  An artifact is written once; a transfer that ends short leaves nothing, so it can be sent again.
- `POST /v1/checkpoints/imports/{import}/commit` names the owner and the full receipt, and `absent` when the Instance is not on this host.
  It records the receipt in the stage and imports it exactly as ADR 0065 does, or reserves recovery authority for the absent Instance as ADR 0066 does.
  A refused commit keeps the stage for a retry or an abort.
- `DELETE /v1/checkpoints/imports/{import}` removes the stage and everything received into it.

The export and receive bodies are streamed by the connection and never held in memory.
The facade is opened only to authorize and open an export; the open descriptors pin the artifacts, so a long stream holds no state-store handle.

What stays as ADR 0064 and ADR 0065 decided:

- the caller owns the object store, its names, its credentials, encryption at rest there, and the publication of a backup;
- the caller keeps a backup unpublished until every artifact matches the receipt's length and digest;
- nothing becomes visible on the host until every artifact verifies against the receipt, and publication is the same atomic rename.

## Consequences

A capture survives the loss of its host: the control plane exports every artifact to the object store, and imports them on a host of the same profile before a same-Instance restore.
ADR 0064's "the command never opens a network listener" and ADR 0065's "creates no new network route" are superseded for the loopback host listener only.
The routes add API minor 1.10.
