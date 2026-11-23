# SOMA API surface

## Status

This page is the documented schema of the SOMA HTTP API that `soma-api` serves under `/v1`.
The `/v1/sandboxes` routes use the existing API vocabulary for creating and managing SOMA instances.
The `{instance}` path parameter is an Instance ID regardless of the software running in the guest.
A test in `crates/soma-api/src/version/tests/surface.rs` generates the surface from the router and the request bodies and fails when it differs from the block below, so the page and the code cannot drift apart.

## Access boundary

The host API at `/v1` accepts a tenant identity from `x-soma-tenant` and validates its format.
Keep it behind a trusted caller or gateway that verifies the client, authorizes the request, and supplies the tenant identity.
The platform runner at `/api/v1` has a separate bearer-key and tenant policy check. [Management-plane security](../../engineering/management-plane-security.md)

## The API version

The API version is `MAJOR.MINOR`, owned by `soma_api::API_VERSION`.
The major names the surface itself: it is the `v1` of every path and the `v1` of the `soma.api.v1` envelope.
The minor counts additive revisions of that surface.

Every response carries the version it was served under in the `soma-api-version` header, including refusals and the `101` of a tunnel.
A client may state the version it needs in the same request header, as `MAJOR` or `MAJOR.MINOR`.
The service refuses an identified request that needs a version it does not serve with `400 api_version_unsupported`, and a header it cannot read with `400 invalid_api_version`, before any route runs.
A request without the header is served as before, so a client written before the header existed keeps working.
An unidentified request is refused as unidentified first, so a caller with no tenant identity learns nothing about the surface.
`soma_api::ApiVersion::serves` is the check a client applies to the header it reads back: the same major, and at least the minor it needs.

`GET /v1/version` returns the API version, the build identity, and every protocol the service speaks, in the `soma.api.v1` envelope with the operation `service.version`.
It needs a tenant identity like every route and no sandbox facade.

```sh
curl -s -H 'x-soma-tenant: operator' http://127.0.0.1:8787/v1/version
```

## Compatibility rule

An addition increments the minor:

- A new route, or a new method on an existing path.
- A new optional request field.
- A new response field, a new error code, or a new value of an open string field.

Anything an older client cannot survive increments the major and is served beside the old major, never in place of it:

- Removing or renaming a route, a method, a request field, or a response field.
- Making an optional request field required, or narrowing what a field accepts.
- Changing the meaning, type, or unit of a field, or the status a refusal is carried under.
- Changing the envelope.

Request bodies refuse unknown fields, so a client cannot send a field a service at an older minor would silently ignore.
A client that needs a new field names the minor that added it in `soma-api-version`.

## What this version does not cover

The public runner surface under `/api/v1` is contract C2: byte-compatible with the platform API it replaces, and versioned by that API.
The runner names its own protocol to the control plane in the `soma-runner-protocol` header of every feed and journal request, as `docs/product/reference/versioning.md` describes.
Response fields and error codes are held to the rule above by each route's integration tests and by review, not by the generated surface.
`VolumeBody` accepts unknown fields because the control plane sends it, so its fields are not part of the generated surface.

## Versions

| Version | Surface digest | What changed |
| --- | --- | --- |
| 1.0 | sha256:0228ba9996e88adacddbaf7b76a2409251b02e3090369f0ea2f4eb5da90d265f | The first explicit version: the surface as it stood when versioning began, plus `GET /v1/version`. |
| 1.1 | sha256:4d51f97ae8a2eb9d45febe33e9fabe26b3d7efb2e731da4306006c5bb23252e0 | Added resident sandbox pause and the paused inspection state. |
| 1.2 | sha256:4980e4a88be7043b08d0545246e4bd13f08bbfccc873b74a5ae3014002697099 | Added resident sandbox resume. |
| 1.3 | sha256:51e8e76a2b494f652d01c0432a93fd4cd6f63ed236f46c003a24784671ed418d | Added resident sandbox checkpoint capture. |
| 1.4 | sha256:72ef81e47b8bcd5f8a99584b709799b0e0482960580921a142b6d7f24f591e2e | Added same-sandbox restore from a published checkpoint. |
| 1.5 | sha256:c5481912fb655a139e58845c65bc4fb912ac0b369622fbc16182f66bb7cf95b3 | Added per-execution environment variables to the loopback command route. |
| 1.6 | sha256:fa26972026e9710aab6687a90ff1fb4a9fcd01adb120917fa2b0d5f2899bbdf0 | Added identity-bound cleanup of a consumed checkpoint. |
| 1.7 | sha256:3818d9888def5ba6aa9470c51a3787a5114042814b97e82166d0af5c0c96aee5 | Added host-local fork with a committed child identity. |
| 1.8 | sha256:808fa360e5a55aa579529c77855446e86052fda23319be920a43b7c8ef220d2c | Integrates encrypted suspension, host capacity reporting, and live autosizing with resident lifecycle operations. |
| 1.9 | sha256:08a30a24261d22a1753fc6aa8447ef4254443061284fb6830007f09f5f1f82d4 | Adds volume-bound checkpoint receipts and same-sandbox mounted restore with reconciliation after broker interruption. |

The upstream development branch numbered these additions 1.1 through 1.3 before integration.
The integrated API continues the resident lifecycle sequence and serves both sets of routes starting at 1.8.

## Surface

The surface of the current version, one fact per line: the API version, the envelope schema and keys, every route as method, path, and operation, and the fields of every request body that refuses unknown fields.
The digest in the table above is the SHA-256 of this block's text.

```text
api 1.9
envelope soma.api.v1 error operation receipt result schema status
route DELETE /v1/sandboxes/{instance} sandbox.destroy
route DELETE /v1/suspensions/{suspension} suspension.destroy
route GET /v1/capacity host.capacity
route GET /v1/prewarm prewarm.status
route GET /v1/sandboxes sandbox.list
route GET /v1/sandboxes/{instance} sandbox.get
route GET /v1/sandboxes/{instance}/autosize sandbox.autosize
route GET /v1/sandboxes/{instance}/resources sandbox.resources
route GET /v1/sandboxes/{instance}/tunnel/{port} sandbox.tunnel
route GET /v1/version service.version
route GET /v1/volumes/{volume} volume.read
route PATCH /v1/sandboxes/{instance}/resources sandbox.resize
route POST /v1/prepared/prune prepared.prune
route POST /v1/sandboxes sandbox.create
route POST /v1/sandboxes/{instance}/checkpoints sandbox.checkpoint
route POST /v1/sandboxes/{instance}/checkpoints/discard sandbox.checkpoint.discard
route POST /v1/sandboxes/{instance}/commands sandbox.command
route POST /v1/sandboxes/{instance}/filesystem/chmod sandbox.filesystem
route POST /v1/sandboxes/{instance}/filesystem/exists sandbox.filesystem
route POST /v1/sandboxes/{instance}/filesystem/list sandbox.filesystem
route POST /v1/sandboxes/{instance}/filesystem/mkdir sandbox.filesystem
route POST /v1/sandboxes/{instance}/filesystem/read sandbox.filesystem
route POST /v1/sandboxes/{instance}/filesystem/read_range sandbox.filesystem
route POST /v1/sandboxes/{instance}/filesystem/remove sandbox.filesystem
route POST /v1/sandboxes/{instance}/filesystem/rename sandbox.filesystem
route POST /v1/sandboxes/{instance}/filesystem/stat sandbox.filesystem
route POST /v1/sandboxes/{instance}/filesystem/write sandbox.filesystem
route POST /v1/sandboxes/{instance}/filesystem/write_range sandbox.filesystem
route POST /v1/sandboxes/{instance}/fork sandbox.fork
route POST /v1/sandboxes/{instance}/pause sandbox.pause
route POST /v1/sandboxes/{instance}/restore sandbox.restore
route POST /v1/sandboxes/{instance}/resume sandbox.resume
route POST /v1/sandboxes/{instance}/stop sandbox.stop
route POST /v1/sandboxes/{instance}/suspend sandbox.suspend
route POST /v1/sandboxes/{instance}/terminal/close sandbox.terminal
route POST /v1/sandboxes/{instance}/terminal/open sandbox.terminal
route POST /v1/sandboxes/{instance}/terminal/read sandbox.terminal
route POST /v1/sandboxes/{instance}/terminal/resize sandbox.terminal
route POST /v1/sandboxes/{instance}/terminal/write sandbox.terminal
route POST /v1/suspensions/{suspension}/resume suspension.resume
route POST /v1/volumes/attach volume.attach
route POST /v1/volumes/destroy volume.destroy
route POST /v1/volumes/detach volume.detach
route PUT /v1/prewarm prewarm.set
route PUT /v1/sandboxes/{instance}/autosize sandbox.autosize
body CheckpointBody operation_id(required)
body AutosizeBody enabled min_cpu max_cpu min_memory_mb max_memory_mb
body ControlBody operation_id
body CreateSandboxBody image volume_id shape name instance_id operation_id
body DiscardBody checkpoint_id(required) state_digest(required)
body FilesystemBody path content recursive to offset length mode create shorten operation_id
body ForkBody operation_id checkpoint_id instance_id image shape name
body PrewarmSetBody targets
body PrewarmTarget reference count memory_mib
body PruneRequest keep apply keep_previous
body ResourcesBody cpu_count memory_mb operation_id
body RestoreBody operation_id checkpoint_id image shape name
body ResumeBody operation_id instance_id image
body RunCommandBody executable arguments limits stdin env operation_id
body SuspendBody operation_id image retention_secs
body TerminalBody columns rows input wait_ms operation_id
```

## Pause

`POST /v1/sandboxes/{instance}/pause` accepts an empty body or an empty JSON object.
A successful response reports `paused` for the same instance, and a repeated pause has the same result.
The runner uses tenant ownership checks for `POST /api/v1/sandboxes/{id}/pause`.
New guest commands receive `409` while the instance remains resident and can be inspected or destroyed.

`POST /v1/sandboxes/{instance}/resume` accepts an empty body or an empty JSON object.
A successful response reports `running` for the same instance, and repeat resume has the same result.
The runner applies its tenant ownership check before forwarding a resume request.

## Restore a stopped sandbox

`POST /v1/sandboxes/{instance}/restore` accepts `operation_id`, `checkpoint_id`, `image`, `shape`, and optional `name`.
The workload fields must identify the original admitted Generation, shape, and name.
The sandbox must have completed a stop and the checkpoint must belong to that sandbox.
A successful response reports `ready` for the same Instance after the worker has authenticated and proved command readiness.
Retry a completed restore with the same operation ID and checkpoint to get its original receipt.
A different operation cannot restore over a live sandbox.

A newly captured checkpoint with a mounted workspace includes `volume.bytes`, `volume.digest`, and `volume.volume_id` in its receipt.
Clients that will release a mounted sandbox for memory-preserving restore require API 1.9 before sending Stop.
Same-instance mounted recovery requires the recorded volume to belong to the stopped instance and be detached on the host's encrypted data mount.
The host stages the saved workspace, verifies the restored worker, and then attaches the restored backing under the same volume ID.
The prior backing retains writes made after the checkpoint and is available to the host's offline rollback transaction.
Older mounted checkpoints without a volume ID remain eligible for independent forks.

The authenticated runner accepts `POST /api/v1/sandboxes/{source}/fork` with `operation_id`, `checkpoint_id`, `image`, and the exact checkpoint source `shape`.
It returns the destination `sandbox_id` and launch receipt once the fork is ready.
Retry with the same operation ID and request on the same source to address the same destination.
The source may be active or stopped.
The request must use the source owner's bearer key and an isolated shape.


## Discard a consumed checkpoint

The trusted host control plane can call POST /v1/sandboxes/{instance}/checkpoints/discard with the published checkpoint_id and state_digest.
It can do so after confirming that Restore succeeded or the sandbox no longer needs that checkpoint.
A matching request returns absent after the checkpoint files are removed and the storage directory is synchronized.
Repeat the same request after a lost acknowledgement.
The route is available on the host-local API and has no equivalent public runner route.

## Fork from a checkpoint

The trusted host control plane sends POST /v1/sandboxes/{instance}/fork with the source checkpoint_id, the original image and shape, an operation_id, a distinct child instance_id, and the recorded source tenant name.
The source must have been created with a tenant name and the child must use the same isolated network shape.
The source can remain running.
A successful reply reports ready for the exact child instance.
Retry with the same values to recover the original receipt.
