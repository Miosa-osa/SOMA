# ADR 0056: The public runner serves the platform's file contract

## Status

Proposed.

## Context

A sandbox created on a public runner answered no file call.
The runner forwarded only soma-api's own loopback routes (`POST /api/v1/sandboxes/:id/filesystem/<op>`, JSON with base64 contents, 4 MiB a call), while every client of the platform calls the platform's routes: `GET /fs?path=`, `GET /fs/read`, `POST /fs/write`, `POST /fs/mkdir`, `POST /fs/stat`, `POST /fs/rename`, `POST /fs/chmod`, `DELETE /fs?path=`, `POST /fs/upload` (multipart), `GET /fs/download`, and the same actions under `files/`.
Each such call on a runner sandbox was `404 not_found` at the runner, and `404 NOT_FOUND` at the control plane, which holds no record of a sandbox the runner minted.
ComputeSDK's file group failed on every SOMA sandbox for that reason.
The guest already performs every operation those routes need (ADR 0051): ranged reads and writes, a detailed listing, make directory, stat, rename, recursive removal, and set mode.
What was missing was the platform's contract on the runner, and a way to move a file larger than one call carries.

## Decision

1. The runner resolves `fs/...` and `files/...` under `/api/v1/sandboxes/:id/` and answers them as the platform's `SandboxFilesController` does: the same parameters (query string and JSON body merged, the body winning), the same path policy, the same success bodies with their keys in Jason's sorted order, and the platform's refusal codes in the platform's `{"code", "error"}` body.
   Routes of that contract it does not serve (`copy`, `write-many`, `tree`, `watch`, `transfer-url`, the legacy `files` upload and download) are answered `501 FILE_ROUTE_UNSUPPORTED` by name, never `404`.
2. The path policy is the platform's: a relative path starts at `/home/user`; a `..` segment, a nul byte, and `~` are refused; what remains must lie under `/home`, `/workspace`, `/tmp`, `/var/log`, `/var/lib`, `/opt`, or `/usr/local` and outside `/etc`, `/proc`, `/sys`, and `/dev`.
   The guest is sent the normalized absolute path, so the bytes it acts on name the path the policy approved.
   The check is on the text, as on the platform; the sandbox boundary remains the machine, and a symbolic link inside it reaches nothing a command in it could not.
3. Every write is published atomically, as the platform's guest daemon publishes one: the bytes fill a temporary sibling (`.soma-write-<random>`), the sibling gets its mode (`0644`, or the requested mode for a file the write creates), and one rename puts it over the target.
   A write that stops part way removes its sibling and leaves the target as it was.
4. A JSON read or write moves at most 32 MiB, as several 4 MiB transfers; a larger file is refused `413 FILE_TOO_LARGE` with the route that moves it.
   Only an admitted key may send a JSON write body that large, because the transport reads a body before the request is authenticated.
5. An upload is streamed.
   For an admitted key the transport hands the body over a bounded channel as it arrives, and does not read it at all until the route takes it, so a refused upload is answered with its body unread and `Expect: 100-continue` holds.
   A streaming multipart reader turns the body into events while holding less than one delimiter of look-behind; the file part is written one 4 MiB transfer at a time, each on a facade lease of its own, and published once the closing delimiter has arrived.
   The destination must be known when the file part begins: a `path` field after the file part that names another destination is refused.
   An upload moves at most 500 MiB, the platform's bound.
6. A download is streamed.
   The file is stat'ed first, so a missing path or a directory is refused before a byte is sent and the answer carries `Content-Length`; the bytes then move one transfer at a time while the sandbox is held.
   A stream whose producer stops before its last byte is aborted by the transport (a reset on HTTP/2 and HTTP/3, a closed connection on HTTP/1.1), never ended, so a client never reads part of a file as the whole of it.
   The peer relay keeps that property across runners, and a streamed upload is never relayed: it is answered `421` with the owner's URL.

## Consequences

- A runner sandbox answers the platform's file calls on the runner's own host, with no control-plane hop.
  The control plane's routing (Caddy and the route shim) still has to send those calls to the runner; that work is outside this repository.
- Known differences from a Firecracker sandbox, each deliberate or forced by what the guest reports: a download is `application/octet-stream` rather than a type guessed from the name; a directory's size is reported as 0; `is_symlink` is always false, because the guest reports a link as neither file nor directory without naming it; a listing that the guest stopped early carries `"truncated": true`; set-user, set-group, and sticky bits are refused rather than set.
- The runner holds at most one 4 MiB transfer per moving file, and a JSON call at most 32 MiB of contents.
- The journal records no file call, as it records no forwarded inspect or terminal call; the paperwork contract is unchanged.
