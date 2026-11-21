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
