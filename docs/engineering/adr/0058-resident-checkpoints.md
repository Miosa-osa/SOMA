# 0058. Resident checkpoint state

Status: Accepted for implementation

## Context

Generation snapshots are produced before a guest receives an Instance identity.
A running sandbox checkpoint can contain tenant data, session state, and credentials in guest memory and in its writable disk.
A generation builder, cache, or restore path must never admit such a checkpoint as a template.

## Decision

Give resident checkpoint state its own magic, schema version, and source Instance identifier.
Reuse the bounded machine header and architecture section codec, and require the resident form to omit the Generation repair-point marker.
Keep the Generation encoder and decoder byte-compatible, including their requirement for the repair-point marker.
Capture the processor, interrupt controller, timer, and clock state from the same paused KVM view used by Generation capture.

The trusted host broker owns checkpoint authorization, staging, and publication.
The VMM may write data only to explicitly handed-off descriptors and must keep arbitrary host paths outside the jail.
A sealed worker receives the bounded request and its descriptors in the same Unix sequenced packet.
The steady-state filter admits the specific KVM state reads needed to take a checkpoint and reserves restore's state writes for startup.
The broker checks the worker's state digest against the staged manifest and publishes only after the worker confirms capture and resume.
Publish a checkpoint receipt only after the memory and disk objects and state manifest are durable and verified.
A capture of a running sandbox must resume that same sandbox before reporting success.
A failed capture must discard uncommitted objects and either resume the original sandbox or report a failure that reflects its actual state.

The SOMA API capture route is `POST /v1/sandboxes/{instance}/checkpoints`.
The caller supplies a canonical operation identity, and a repeat on the same live Instance returns its published receipt without recapturing.
The public runner checks tenant ownership before handing off the request, and the raw host listener is a trusted loopback interface.
A host with no configured private checkpoint directory, or a worker without descriptor capture support, refuses the operation.

Restore must validate the expected tenant, source Instance, Generation and artifact digests, and machine compatibility before mapping any checkpoint bytes.
An ordinary Generation restore may never read resident checkpoint state.
Same-instance recovery and fork have separate authority and identity requirements.
Restoring or forking may not duplicate host handles, guest control credentials, or network authority.

## Same-Instance restore

The broker supplies the expected Instance and Generation IDs, the published state digest, and private open artifacts.
The resident entry point checks compatibility and hashes the state, RAM, immutable root, and private writable head against the captured records before creating the VM.
ADR 0068 lets it skip the RAM and root hashes when the broker attests those unchanged files' exact stamps.
It verifies that the private writable head is a different inode from the saved overlay.
The restored VM retains the Instance identity and starts without network authority.
The prepared VM is not an authenticated session or a Ready result.
A worker must publish fresh reconnect material and verify an authenticated command before reporting readiness.

The adjacently stored guest agent must support authenticated reconnect before the runtime can accept a restored Instance.
When its previous idle transport closes, the agent checks once for a new launch page that the broker installed before the vCPU ran.
The page must name the same Generation and Instance and a new operation.
The agent rotates entropy and corrects wall time, preserves the guest's files and process state, and authenticates a fresh session.
An absent page, an invalid binding, or any handshake failure ends the guest.
Protocol and authentication failures on the old channel never enter reconnect.

The VMM worker admits this path as a separate source and enters the same authenticated command loop used by ordinary sandboxes.
It refuses an Instance, Generation, or CID different from the authorized restore request and refuses a network attachment or new launch secrets on this path.

The sealed-worker control request names recovery explicitly and carries the broker's expected state digest.
The state, RAM, root, and new writable head use the existing attested descriptor slots.
When a writable head exists, the original overlay descriptor travels with the recovery request so the worker can reject aliasing independently.
The worker only accepts recovery as its first launch and must confirm the fresh command session before the broker seals it.

## Managed restore admission

A managed restore request names the existing Instance, a published checkpoint operation, and a distinct restore operation.
The engine admits it only from a confirmed stopped record for the same workload and machine shape.
The durable store's revision comparison grants a single restore owner before contacting the machine host.
The restored worker becomes addressable under the same Instance after the host confirms readiness.
A duplicate must never initiate another launch or clean up an in-progress restore.
The host launch handoff carries a checkpoint operation ID, never an artifact path.
The host must reject recovery if it cannot use an isolated sealed worker, and older hosts must reject the unfamiliar handoff rather than perform a fresh launch.
A restore skips ordinary post-launch warmup and begins without a claimed network, attached volume, or new launch secrets.
The trusted host listener exposes restore as `POST /v1/sandboxes/{instance}/restore` with the checkpoint ID, restore operation ID, and workload request.
The host compares the resolved Generation and workload request to the stopped record before opening the checkpoint.

## Retention

A published checkpoint keeps its stopped Instance record and its exact prepared Generation eligible for restore.
The terminal record sweep consults the checkpoint directory before releasing a stopped record, and prepared-Generation pruning includes each published checkpoint as a holder.
Both checks operate on atomically published directories and fail closed when checkpoint metadata cannot be read.
Files still being staged do not hold a terminal record or a Generation.

## Interrupted restore

A restore holds an exclusive host-local lease from admission through durable Ready publication.
The launcher gives the same lease to the new host process with the checkpoint handoff so the lease cannot disappear if the API exits before that host binds its Instance socket.
The host releases its copy only after reporting Ready or completing cleanup.
Once a new caller holds the lease, an exact retry may reap the abandoned host and restore from the same checkpoint and stopped record.
A different operation cannot replay or take over an interrupted restore.

## Authenticated runner

The public runner exposes `POST /api/v1/sandboxes/{id}/restore` using its bearer-key and tenant checks.
It supplies the tenant label from the authenticated principal, and the engine compares that label with the stopped record before admission.
If the runner lost its in-memory entry, it checks the durable owner and reserves a tenant slot before attempting restore.
A successful restore returns the sandbox to the runner's normal lifetime tracking.

## Fork a resident checkpoint

A fork has an authorized source Instance and a distinct, newly allocated destination Instance.
The checkpoint's manifest must match the source and the prepared Generation, and the destination receives its own private writable disk and copy-on-write RAM.
The fork launch page uses schema 4 with the source Instance covered by the page digest.
Ordinary launches and same-Instance restores continue to use schema 3.
An older guest agent rejects the fork page.
The guest accepts a fork only after losing its captured idle control connection and only when the page names that captured Instance and Generation.
It applies the destination hostname and machine ID, refreshes entropy and time, and authenticates a new control session.
The guest keeps its workload processes, files, and scratch mounts.
Host network authority and attached volumes are unavailable to this fork path.

The sealed worker uses a distinct `fork` request naming the authorized source and the new launch, with the receipt digest.
The broker opens the artifacts under the source identity and builds a private disk for the destination identity.
Older workers reject the unfamiliar request before running any VM.

The host handoff carries a fork source Instance only alongside the checkpoint operation and the destination lease.
A fork with a missing checkpoint or a source equal to its destination is refused before creating an Instance socket.
An older host rejects the unfamiliar fork field instead of treating it as a same-Instance restore.
A primed host cannot accept either checkpoint operation because the handoff for a primed host carries no recovery lease.

The managed engine validates the source's durable owner, workload identity, Generation, and shape before admitting the destination.
It accepts an active source or a source with a confirmed Stop receipt.
The destination lease covers its own write-ahead record and hosted launch; the source remains untouched.
A fork request fingerprint binds the ordinary destination launch to the source and checkpoint.

The authenticated runner exposes `POST /api/v1/sandboxes/{source}/fork` on the source host.
The destination ID is derived from the tenant, source, and fork operation ID so a replay addresses the original destination.
A new destination reserves a tenant slot and a replay of an active destination holds its existing slot.
The server derives the durable owner label from the authenticated principal and accepts no owner label in the request body.
