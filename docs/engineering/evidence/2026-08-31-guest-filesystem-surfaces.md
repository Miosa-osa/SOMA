# The guest filesystem reached from the public surfaces - 2026-08-31

## Evidence boundary

This run proves that the six filesystem operations the provider contract names reach a real
`node:22` guest, on real KVM, through both surfaces a caller uses: the HTTP service a ComputeSDK
client talks to, and the command line. Before it, `soma-guest` implemented the operations at the
protocol level and no public call could reach any of them; every filesystem route answered
`501 capability_unavailable`.

Proved live by this run:

- Through the HTTP API: a sandbox created, a directory made in it, a file written, the same bytes
  read back, the file listed in its directory, reported present, removed, and then reported
  absent. Each was a separate request against a sandbox held by a host process that outlives all
  of them.
- The same six through `soma machine file`, as sixteen separate processes against one sandbox
  that outlives every one of them.
- A binary round trip on both surfaces: twelve bytes that are not valid UTF-8 anywhere in them,
  written and read back identical by SHA-256. On the command line this holds for the JSON
  envelope and for a plain redirect of the human output to a file.
- Six failures, each returning a typed cause rather than a leaked host error: a path the guest
  protocol will not carry, a read of something absent, a write the guest denied, a write it could
  not complete for another reason, a file read of a directory, and a non-empty directory removed
  without consent.

Raw envelopes and the drivers are under
[`raw/2026-08-31-guest-filesystem-surfaces/`](raw/2026-08-31-guest-filesystem-surfaces/).

**What this run does not prove.** MCP was not driven. It reaches the same engine call through the
same facade as the other two and is covered by workspace tests, but it has no live proof and
should not be described as having one. Nothing here is a latency observation. Nothing here says
anything about concurrency, about more than one host, or about files larger than the bound one
call moves.

## Identities

- SOMA Git revision: `8512223`, whose code is what the run was made on. It was synchronised to
  eval-1 at `/srv/soma/guestfs/repo`, and the digest of every `.rs` file under `crates/` was
  compared between the two before this record was written, so the run revision is checked rather
  than asserted. One later commit on this branch moves a function between modules inside
  `soma-mcp`, which no surface proved here reaches.
- Host: eval-1, `Linux 6.8.0-138-generic` x86_64, 80 threads, XFS scratch at `/srv`.
- Dates: the Generation was prepared and the HTTP session run on 2026-08-31 UTC; the command-line
  session and the read-only probe were run on 2026-09-01 UTC, after the host became reachable
  again. All three ran against the same store, at the same revision, on the same host.
- Rust toolchain: `rustc 1.98.0 (88d9e12ae 2026-08-18)`, release profile.
- Kernel: `vmlinux-6.12.107-soma-v1`, SHA-256
  `f1af3a142fa39916cfac425a01b16b5f328279823533421c9eec3f192c05b746`.
- Guest agent: the `x86_64-unknown-linux-musl` release build of this tree, SHA-256
  `13cb1b44a0478ffe74031f130d731ecbd672078aebe4a4c1133459a9bcfad485`.
- Generation: `node:22` compiled and captured by `scripts/reproduce.sh` at one vCPU, 1024 MiB of
  memory and 10240 MiB of storage, which is the shape every launch below asks for. Wire contract
  fingerprint `368f81b2e540ab97`. The steps are in
  [`reproduce.txt`](raw/2026-08-31-guest-filesystem-surfaces/reproduce.txt) and the run's own
  output in [`prepare.log`](raw/2026-08-31-guest-filesystem-surfaces/prepare.log).
- Instances: `50d2b493a9f54bdb94e9925b80e9c88d` for the HTTP session,
  `8ef411455c8c4cc49a849fcadab8f90d` for the command line, and
  `06f8df6bdc0c4ed48dd0613ab5f29cdb` for the read-only probe. Each was created and destroyed by
  the run that used it.

## What was missing

`crates/soma-guest` carries eight filesystem operations over the guest protocol, with a closed
failure set and a redacting `Debug`, and they work: they were proved against a real guest at
`8ec0119` ([Guest capabilities live](2026-08-31-guest-capabilities-live.md)). What did not exist
was any way to ask for one. The portable facade's `Backend` trait carried resolve, launch,
execute, inspect and cleanup, so no engine call reached them, and `crates/soma-api` said so in
its own words:

    the SOMA portable facade exposes no guest filesystem transfer;
    the guest protocol implements one, but no backend or engine method reaches it

## Two defects this run found before it could run at all

Neither was in the filesystem path. Both stopped the HTTP service from creating any sandbox a
second request could address, so both had to be fixed before the first operation could be tried.

1. The service never asked for hosted machines, so every request opened a runtime whose machine
   died with the connection that made it.
2. The runtime starts a host by re-entering the current executable with `machine-host`, which
   only the command line answered. The service spawned a copy of itself, which parsed that as an
   option it does not have and exited. A create call reported `backend_unavailable` with no
   machine anywhere.

The first `POST /v1/sandboxes` of this run, before the fixes, is the second of those:

    {"schema":"soma.api.v1","operation":"sandbox.create","status":"error","result":null,
     "error":{"code":"backend_unavailable","message":"backend capability is unavailable",
     "retryable":true},"receipt":null}

## The six operations

Each row is one HTTP request against the sandbox created by the first.
[`http/`](raw/2026-08-31-guest-filesystem-surfaces/http/) holds, for each exchange, the request
exactly as it was sent, the response body exactly as it came back, and the status code.

| Operation | Route | Status | What the guest answered |
| --- | --- | ---: | --- |
| create | `POST /v1/sandboxes` | 201 | instance `50d2b493…c88d`, `state: ready` |
| make directory | `POST …/filesystem/mkdir` | 200 | `operation: mkdir`, nothing else to report |
| write | `POST …/filesystem/write` | 200 | `byte_length: 20` |
| read | `POST …/filesystem/read` | 200 | 20 bytes, base64 `aGVsbG8gZnJvbSB0aGUgaG9zdAo=` |
| list | `POST …/filesystem/list` | 200 | one entry, name `hello.txt`, `kind: file`, `more_entries: false` |
| exists | `POST …/filesystem/exists` | 200 | `exists: true`, `kind: file` |
| remove | `POST …/filesystem/remove` | 200 | `operation: remove`, nothing else to report |
| exists, after the removal | `POST …/filesystem/exists` | 200 | `exists: false`, and no `kind` at all |

The last row is the one that makes the sequence a proof rather than a set of calls that each
returned 200: the same request that reported the file present reports it absent once it was
removed, and the answer document drops `kind` rather than carrying a stale one.

A read answers with the file's bytes and a listing answers with each entry's own name, both
base64 with their decoded length beside them, which is the shape this service already uses for
command output. Names are bytes for the same reason paths are: a guest name is not required to be
UTF-8, and `hello.txt` above is `aGVsbG8udHh0` rather than a string.

## The same six on the command line

Sixteen separate `soma` processes against one sandbox, none of which holds it.
[`cli/`](raw/2026-08-31-guest-filesystem-surfaces/cli/) retains the command, the JSON envelope,
the standard error and the exit status of each.

| Process | Command | Exit | Result |
| --- | --- | ---: | --- |
| 1 | `machine launch node:22` | 0 | instance `8ef41145…f90d`, `state: ready` |
| 2 | `machine file mkdir --instance-id … /workspace/cliproof` | 0 | `operation: mkdir` |
| 3 | `machine file write --instance-id … --content-file hello.txt /workspace/cliproof/hello.txt` | 0 | `byte_length: 27` |
| 4 | `machine file read --instance-id … /workspace/cliproof/hello.txt` | 0 | 27 bytes, `aGVsbG8gZnJvbSB0aGUgY29tbWFuZCBsaW5l` |
| 5 | `machine file list --instance-id … /workspace/cliproof` | 0 | one entry, `hello.txt`, `kind: file` |
| 6 | `machine file exists --instance-id … /workspace/cliproof/hello.txt` | 0 | `exists: true`, `kind: file` |
| 7 | `machine file remove --instance-id … /workspace/cliproof/hello.txt` | 0 | `operation: remove` |
| 8 | `machine file exists --instance-id … /workspace/cliproof/hello.txt` | 0 | `exists: false`, no `kind` |
| 16 | `machine destroy --instance-id …` | 0 | `state: destroyed` |

A write takes its bytes from a host file rather than from an argument, because an argument would
have to be text and text cannot carry a file that is not valid UTF-8. A read writes the file's
bytes to standard output and nothing else, so a plain redirect reproduces the guest's file; that
is the third digest in the binary section below.

A refusal is a non-zero exit here, unlike the HTTP surface where it is a 200. A script that wrote
into a location the guest declines must not see success, so the envelope reports
`filesystem_refused` and the process exits 69 while still carrying the typed cause in its result:

    {"schema":"soma.cli.v1","command":"machine.file","status":"error",
     "result":{"instance_id":"8ef411455c8c4cc49a849fcadab8f90d","operation":"read",
     "refusal":"not_found"},
     "error":{"code":"filesystem_refused","message":"the guest declined the filesystem operation",
     "retryable":false},"receipt":null}

An inadmissible path never reaches the engine and exits 65 as ordinary invalid input, with code
`invalid_guest_path`.

## Binary round trip

Text-only file transfer that corrupts binaries is a quiet defect, so the bytes chosen are ones a
text transfer cannot survive:

    00 ff fe 80 0a 7f c3 28 00 ed a0 80

None of that is valid UTF-8. `0xff` and `0xfe` never appear in UTF-8 at all, `0x80` is a
continuation byte with no leader, `c3 28` is a leader followed by a byte that cannot continue it,
and `ed a0 80` encodes the surrogate U+D800, which UTF-8 forbids. Decoding this as text and
re-encoding it would replace eight of the twelve bytes.

Over HTTP, written through `…/filesystem/write` and read back through `…/filesystem/read`
([`http/binary-verdict`](raw/2026-08-31-guest-filesystem-surfaces/http/binary-verdict)):

    BINARY ROUND TRIP: identical
    ddd1a4a8786aa89014b0d058cf6f4b9b2e7fa0c33f4677fbaa7ecac3e7ce66cf  binary.bin
    ddd1a4a8786aa89014b0d058cf6f4b9b2e7fa0c33f4677fbaa7ecac3e7ce66cf  binary-returned.bin

Over the command line, twice: once through the JSON envelope's base64 `content`, and once through
a plain shell redirect of the human output
([`cli/binary-verdict`](raw/2026-08-31-guest-filesystem-surfaces/cli/binary-verdict)):

    BINARY ROUND TRIP (json envelope): identical
    BINARY ROUND TRIP (human redirect): identical
    ddd1a4a8786aa89014b0d058cf6f4b9b2e7fa0c33f4677fbaa7ecac3e7ce66cf  binary.bin
    ddd1a4a8786aa89014b0d058cf6f4b9b2e7fa0c33f4677fbaa7ecac3e7ce66cf  binary-returned.bin
    ddd1a4a8786aa89014b0d058cf6f4b9b2e7fa0c33f4677fbaa7ecac3e7ce66cf  binary-human.bin

`binary.bin` is what the host wrote, `binary-returned.bin` is the read response's base64 `content`
decoded back to bytes, and `binary-human.bin` is `soma machine file read` redirected straight to a
file. The SHA-256 of those twelve octets is `ddd1a4a8…7ce66cf`, which is what all four report.

The redirect case is the one that would have hidden a defect. It is the only path where the bytes
are not carried by an encoding that is obviously binary-safe, so it is the one where a stray
newline, a lossy decode, or a `println!` of a string would have shown up.

## Failures

Every one returned a cause from the closed set the guest protocol defines, or was refused before
the wire. None returned a host errno, a host message, or a guest path. Both surfaces were driven
through the same five cases and answered with the same causes.

| Case | Request | HTTP | Command line | Cause |
| --- | --- | ---: | ---: | --- |
| a path outside the permitted tree | read `relative/escape` | 400 | exit 65 | refused before the wire |
| a read of something absent | read `…/never-written` | 200 | exit 69 | `not_found` |
| a write to a read-only location | write `/proc/version` | 200 | exit 69 | `failed` |
| a file read of a directory | read `/workspace/proof` | 200 | exit 69 | `wrong_kind` |
| a non-empty directory removed | remove `/workspace/proof` | 200 | exit 69 | `not_empty` |

Three of these deserve their reasoning stated rather than assumed.

**The inadmissible path is refused before the wire, and that is not a shortcut.** An empty,
relative, oversized, or nul-bearing path is not a request the guest declines: the guest rejects
it while decoding, which is a protocol fault that ends the session. A caller naming one would
have destroyed its own sandbox instead of being told no. So the facade refuses it first, with the
same rule the protocol states, and a test in `soma-local` holds the two equal by running
candidate paths through the protocol's own encode and decode. The guest still performs the check
itself and remains the authority; nothing on the host resolves, normalises, or approves a path,
and what is admissible beyond this shape is still decided inside the guest.

**A refusal is a 200 over HTTP and a non-zero exit on the command line, deliberately.** The
operation reached the guest and the guest declined it, so reporting it as a transport or service
failure would tell an HTTP client something untrue about where the answer came from. A shell
script has no result document to read before it continues, so there the same answer has to be a
failing exit or a script would carry on as though the write had happened. Both carry the typed
cause in the result either way.

**The read-only write answered `failed` rather than `denied`, and the follow-up says why.** The
guest maps `EACCES`, `EPERM` and `EROFS` to `denied` and everything else to `failed`. Writing to
`/proc/version` produces none of those three: it is a procfs file with no write handler. That row
therefore proves a typed cause rather than a leaked host error, which is what the closed set is
for, but on its own it left the `denied` mapping unproved through a public surface. A second
probe settled it, and is retained at
[`readonly/`](raw/2026-08-31-guest-filesystem-surfaces/readonly/).

It first asked the guest what it mounts. Every mount is `rw`, and the root is an overlay with a
writable upper over the EROFS lower:

    overlay / overlay rw,nosuid,nodev,relatime,lowerdir=/mnt/lower,upperdir=/mnt/upper/upper,...
    proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0
    sysfs /sys sysfs rw,nosuid,nodev,noexec,relatime 0 0

So there is no read-only mount in this guest, `EROFS` is not reachable from any guest path, and
"a read-only root" is not a thing this sandbox has: the root is writable by design. Writing one
byte into five locations then gave:

| Path | Answer |
| --- | --- |
| `/proc/version` | `failed` |
| `/sys/kernel/vmcoreinfo` | `denied` |
| `/proc/sys/kernel/hostname` | wrote 1 byte |
| `/etc/hostname` | wrote 1 byte |
| `/workspace/plain.txt` | `not_found` |

`/sys/kernel/vmcoreinfo` is the case that proves `denied` live through the public surface: the
guest refuses to open it for writing, which is `EACCES` or `EPERM`, and the caller is told
`denied` rather than an errno. The two writes that succeeded are worth keeping in the record
too, because they are what makes the first row's `failed` the honest answer rather than a
missing mapping: the guest agent runs as root and the root filesystem is writable, so an
ordinary path is not denied to it, and the locations that do refuse refuse for reasons other
than a read-only mount.

## What now happens

`Backend` carries a sixth method, and the engine has the use case that drives it. The KVM Backend
serves it on both paths a machine can be held on: the resident one, where the process owns the
session, and the hosted one, where the process holding the machine is addressed over its socket.
Every operation above crossed both, because the service holds its machines in host processes.

The two operations that move a whole file use the chunk loop the guest session already exposes,
because one record carries a bounded body and a whole file does not fit in one. Nothing new was
added to frame them.

The `GuestFilesystemTransfer` refusal stays, and now names what is actually absent rather than a
gap that is closed: a backend whose machines do not outlive the process that launched them has no
sandbox for a later filesystem call to reach. That is the Docker and Apple Virtualization case,
and it is the answer those backends give.

## Open

- MCP has no live proof. It is wired to the same engine call and covered by workspace tests, but
  it has not been run against a guest.
- `Create` and `SetMode` exist on the guest protocol and no surface exposes them. There is no
  rename, no copy, no recursive listing, and no transfer larger than the four mebibytes one call
  moves.
- The four mebibyte bound is a decision, not a measurement. It exists because a hosted machine
  relays the operation to the process holding it as one bounded JSON line, where a byte becomes
  up to four characters, and the bound has to leave the relayed form inside the line ceiling that
  path already enforces. A caller that needs a larger file has no way to ask for one today.
- One host, one Generation, one sample of each operation. Nothing here was run twice.
