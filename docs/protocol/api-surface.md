# SOMA API surface

## Status

This page is the documented schema of the SOMA HTTP API that `soma-api` serves under `/v1`.
A test in `crates/soma-api/src/version/tests/surface.rs` generates the surface from the router and the request bodies and fails when it differs from the block below, so the page and the code cannot drift apart.

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
The runner names its own protocol to the control plane in the `soma-runner-protocol` header of every feed and journal request, as `docs/versioning.md` describes.
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

## Surface

The surface of the current version, one fact per line: the API version, the envelope schema and keys, every route as method, path, and operation, and the fields of every request body that refuses unknown fields.
The digest in the table above is the SHA-256 of this block's text.

```text
api 1.4
envelope soma.api.v1 error operation receipt result schema status
route DELETE /v1/sandboxes/{instance} sandbox.destroy
route GET /v1/prewarm prewarm.status
route GET /v1/sandboxes sandbox.list
route GET /v1/sandboxes/{instance} sandbox.get
route GET /v1/sandboxes/{instance}/resources sandbox.resources
route GET /v1/sandboxes/{instance}/tunnel/{port} sandbox.tunnel
route GET /v1/version service.version
route GET /v1/volumes/{volume} volume.read
route PATCH /v1/sandboxes/{instance}/resources sandbox.resize
route POST /v1/prepared/prune prepared.prune
route POST /v1/sandboxes sandbox.create
route POST /v1/sandboxes/{instance}/checkpoints sandbox.checkpoint
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
route POST /v1/sandboxes/{instance}/pause sandbox.pause
route POST /v1/sandboxes/{instance}/restore sandbox.restore
route POST /v1/sandboxes/{instance}/resume sandbox.resume
route POST /v1/sandboxes/{instance}/stop sandbox.stop
route POST /v1/sandboxes/{instance}/terminal/close sandbox.terminal
route POST /v1/sandboxes/{instance}/terminal/open sandbox.terminal
route POST /v1/sandboxes/{instance}/terminal/read sandbox.terminal
route POST /v1/sandboxes/{instance}/terminal/resize sandbox.terminal
route POST /v1/sandboxes/{instance}/terminal/write sandbox.terminal
route POST /v1/volumes/attach volume.attach
route POST /v1/volumes/destroy volume.destroy
route POST /v1/volumes/detach volume.detach
route PUT /v1/prewarm prewarm.set
body CheckpointBody operation_id(required)
body ControlBody operation_id
body CreateSandboxBody image volume_id shape name instance_id operation_id
body FilesystemBody path content recursive to offset length mode create shorten operation_id
body PrewarmSetBody targets
body PrewarmTarget reference count memory_mib
body PruneRequest keep apply keep_previous
body ResourcesBody cpu_count memory_mb operation_id
body RestoreBody operation_id checkpoint_id image shape name
body RunCommandBody executable arguments limits stdin operation_id
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

The authenticated runner accepts `POST /api/v1/sandboxes/{source}/fork` with `operation_id`, `checkpoint_id`, `image`, and the exact checkpoint source `shape`.
It returns the destination `sandbox_id` and launch receipt once the fork is ready.
Retry with the same operation ID and request on the same source to address the same destination.
The source may be active or stopped.
The request must use the source owner's bearer key and an isolated shape.
