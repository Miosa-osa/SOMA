# ADR 0051: Guest API level, stat, rename, and ranged files

## Status

Proposed.

## Context

The guest file protocol moved whole files only, could not say how big a file is, and could not replace a file atomically.
A caller that edits a large file, or publishes a configuration file safely, needs a byte range, a size, and a rename.
Adding a request tag is not safe by itself: a guest that does not know a tag treats it as a protocol fault and ends the session.
So a host must know what the guest can do before it sends a new tag, and no rollout order may make a mixed pair fail.

## Decision

1. The file protocol gains `Stat` and `Rename` requests and a `Stat` outcome (kind, size, permission bits, modification time in seconds).
   `SetMode` already exists on the wire and is now reachable.
   Ranged reads and writes need no new tag: they are the existing offset reads and writes with a caller-chosen length.
2. The facade gains `Stat`, `Rename`, `ReadRange`, `WriteRange`, and `SetMode` operations, `Range` and `Stat` answers, and a `FileRefusal::Unsupported` cause.
   A ranged operation is bounded by the same 4 MiB as a whole-file transfer.
3. The host learns the guest's level with a probe that an older guest cannot mistake.
   It reads `/.soma/guest-api/<nonce>`, where the nonce is fresh random alphanumeric text chosen per session.
   A level 2 agent answers from memory, never from disk, with `"<level> <nonce>\n"`.
   The host accepts only that exact echo.
   Any other answer is level 1: a failure, a plain file, or file content planted at the path by a tenant.
4. The probe is lazy and cached for the session.
   It runs on the first level 2 operation, so a session that never uses one sends nothing new.
5. Against a level 1 guest a level 2 operation is answered `Unsupported` before anything is sent.
   The session is untouched and an upgraded guest answers the same request.
6. Paths (including the rename destination), modes, and range lengths are checked on the host before the wire, so a bad request is a refusal and never a protocol fault.
7. Both new tag numbers are final and never reused: request `Stat` is 9 and `Rename` is 10, outcome `Stat` is 7.

## Consequences

- Any mix of host and guest works.
  Old host with new guest never sends a level 2 word.
  New host with old guest refuses cleanly.
  New with new works.
- The write path for the persisted environment can now be atomic: write a temporary file, then rename it over the target.
- A hostile workload cannot forge level 2 on an old guest, because it cannot know the nonce before the host sends it.
- A host that talks to a guest must keep one probe's worth of state per session.
- An older worker or host that receives a new facade operation fails to decode it and refuses it, rather than ignoring fields.
