# Interactive terminal and sandbox enumeration: unfinished work - 2026-09-01

**This is not evidence. Nothing here was run against a guest.** It is a handover note for the two
provider-contract gaps that `docs/research/provider-contract-gap-analysis.md` still lists as
missing, written at the point the session ended. The code is on `feat/pty-and-list` at `dc115dc`,
based on `70530c2`.

## What is done

Both gaps are wired end to end at the code level, following the shape the guest filesystem work
established at `70530c2` rather than inventing a second one.

- `cargo check --workspace --all-targets` passes.
- `cargo clippy --workspace --all-targets` is clean.
- `cargo fmt --all` has been run.
- `cargo test --workspace` passes except the two tests named under **What is not done**.

### The terminal

`crates/soma-guest/src/application/pty/` already implemented the protocol and
`crates/soma-guest-agent/src/pty.rs` already served it. What did not exist was any way to ask for
one. Added, mirroring the filesystem path exactly:

- `soma::PtyOperation` / `PtyAnswer` / `PtyRefusal` / `PtyObservation`, with the guest's closed
  failure set carried across unchanged and a `Debug` that never prints a byte typed at or produced
  by the terminal.
- `Backend::pty`, and `Engine::pty_machine`, which admits with the same read the filesystem uses
  (renamed to `admit_operation` and now shared): no receipt, no tombstone, no phase transition.
- The KVM backend on both paths: `pty_resident` for the process that owns the session, and
  `host::pty` over the Instance socket for a hosted machine. The portable operation crosses the
  relay as itself, so a client cannot choose a guest request the mapping would not have produced.
- `POST /v1/sandboxes/{id}/terminal/{open,write,read,resize,close}` and `soma machine pty
  {open,write,read,resize,close}`.
- `crates/soma-local/tests/guest_pty_bound.rs`, which holds the facade's restated terminal bounds
  equal to the protocol's by running candidates through the protocol's own encode and decode. It
  passes. Widening `PtyRequest::{encode_body,decode_body}` to `pub` was needed for this and
  matches what `FileRequest` already does for the same reason.

### Enumeration

- `StateStore::list` returns the identities a store holds. `FileStateStore` implements it as a
  directory read in `file_store/enumerate.rs`, taking no lock; a name that is neither an Instance
  identity nor the store's own `.locks` directory is `Corrupt` rather than skipped.
- `Engine::list_machines` reads each record back under its own lock before reporting it, skips a
  record that reached a terminal phase, and asks the backend for liveness.
- `MissingCapability::SandboxEnumeration` is gone. `GuestTerminalSession` was added for the
  backends that cannot hold a session between two requests.

## What is not done

- **No live proof.** Nothing here has run on eval-1 or against any guest. This is the whole
  remaining job and it is what the next person should do first.
- **Two `soma-api` refusal tests fail** and are the only failing tests in the workspace:
  `listing_sandboxes_never_answers_with_an_empty_collection` and
  `listing_sandboxes_refuses_and_names_the_missing_store_capability`, both in
  `crates/soma-api/tests/refusals.rs`. They assert the 501 that `sandbox.list` used to return.
  They need rewriting to assert the new document, not deleting: the first one's intent (an empty
  list must never be indistinguishable from a refusal) is still the right thing to hold, and is
  now held by the `host` member rather than by a 501.
- **MCP has no terminal tool and no list tool.** The other surfaces are wired; `soma-mcp` was not
  touched. It should follow `crates/soma-mcp/src/file.rs` exactly.
- **No unit tests were written for the new engine paths.** The test doubles are in place and
  usable: `crates/soma/tests/support/terminal.rs` and `crates/soma-api/tests/support/terminal.rs`
  each hold one echoing session, and `FakeFacade::holding` takes a set of sandboxes for a listing.
  `SandboxEntry::new` is public so a non-engine facade can build one.
- `docs/claim-ledger.md` and `docs/research/provider-contract-gap-analysis.md` are untouched, and
  must stay that way until there is live proof. The gap analysis is currently correct.

## Findings worth more than the code

### The devpts hazard is already fixed, and it is worth not re-investigating

The recorded hazard was that devpts was never mounted in the guest, which made the terminal
unreachable in a real guest even though the protocol worked. That is fixed on `main`.
`crates/soma-guest-agent/src/boot.rs:100-109` mounts `devpts` on `/dev/pts` with
`mode=0620,ptmxmode=0666`, and the comment there states the reason: `/dev/ptmx` cannot allocate a
pair without it. `boot.rs:151` also keeps device nodes usable across the later remounts.
`crates/soma-guest-agent/src/pty/device.rs` opens `/dev/ptmx` and then `/dev/pts/<n>`.

So the wiring really was the only gap. **This is still unverified live.** The first live check
should be `tty` inside the guest through the new surface, because a pipe answers `not a tty` and a
real pseudo-terminal answers `/dev/pts/0`, and that single answer is what separates this from a
capability that merely compiles.

### The four-mebibyte relay bound does not apply to a terminal, and this was a deliberate decision

The filesystem work flagged that a PTY stream would hit `MAX_FILE_BYTES` harder than a file does.
It does not, and the reason is worth stating rather than assuming.

`MAX_FILE_BYTES` is 4 MiB because a hosted machine relays the operation as one JSON line where a
byte becomes up to four characters, and a whole file has to be held entire. A terminal is
incremental by nature: `MAX_PTY_CHUNK_BYTES` is 4096, which is the *guest protocol's* own record
bound, not a bound this side chose, and **nothing on the host accumulates a session's output
across calls**. One terminal call is one guest record in each direction. The relay's own line
ceiling (`MAX_LINE_BYTES`, 64 MiB) is four orders of magnitude clear of it. The bound a terminal
lives under is therefore the guest's, and raising it would be a guest-protocol change rather than
a relay one.

### A terminal is a stream and this is not one, on purpose

Every transport SOMA has carries one addressed request and one bounded answer: an HTTP request, a
CLI process, an MCP tool call, and the single JSON line the machine host relays. The session lives
in the guest for as long as the machine does, so a caller drives it with bounded calls and the
terminal keeps running between them. A read carries how long it may wait for its first byte, so a
caller with nothing to read blocks in the guest rather than spinning.

Rejected, and why:

- **A streaming HTTP endpoint or a WebSocket.** It needs a second transport on every surface, and
  neither the CLI nor MCP can consume one. It would have made the terminal reachable from exactly
  one surface.
- **Relaying a raw guest PTY frame through the machine host.** Same objection the filesystem work
  recorded: a client could then choose a request the mapping would not have produced.
- **Buffering output on the host between calls.** It would make the host hold unbounded tenant
  data and would make two processes reading the same terminal disagree about what they had seen.

The one thing this shape does not give you is a terminal that pushes. A caller polls with a wait.
That is a real limitation and it should be stated in any evidence document, not glossed.

### What `list` should honestly return, and why it is two facts and not one

This is the part that most deserves to survive.

A durable record says an Instance was admitted and that nothing has released it. **It cannot say
whether the process holding that Instance's machine is still running**, because nothing writes to
the record when a process dies: a host killed with `SIGKILL`, or lost with the whole machine,
leaves an `Active` record behind and no machine anywhere. The two sources of truth named in the
brief -- the durable records, and the sockets under `<state-root>/machines/` -- answer different
questions, and picking either one alone produces a lie.

- Records alone report a dead sandbox as live. That is exactly the class of false success this
  repo spent 2026-08-31 removing.
- Sockets alone cannot report a sandbox mid-launch, cannot name its backend or its shape, and
  cannot distinguish an Instance that never existed from one that was destroyed cleanly.

So `SandboxEntry` carries both, side by side, separately named, and never collapsed:

- `state` (`launching` / `active` / `executing` / `terminating`) is what the durable record says
  the last completed transition left it in.
- `host` (`live` / `absent` / `unknown`) is what the backend could reach at the moment of the
  listing.

`SandboxLiveness::Unknown` is load-bearing and must not be optimised away. A backend that holds
machines inside the launching process cannot tell whether a record written by some other process
names a machine that died with it, so it says so. Docker says `unknown` too: its containers are
registered with a daemon, and this backend has no probe cheaper or more certain than a full
inspection.

Three decisions inside that, each of which could reasonably have gone the other way:

1. **A destroyed sandbox is not in the listing at all.** Its terminal record is durable evidence
   that it existed and was released, and it is still readable by exact identity, but a list of
   sandboxes that included destroyed ones answers a question nobody asked.
2. **A sandbox whose host died *is* in the listing, labelled `host: absent`.** Dropping it would
   be a false absence: the record is still there, it still holds an identity, and it has not been
   released, so something still has to clean it up. A ComputeSDK client that wants only usable
   sandboxes filters on `host == "live"`, and the document makes that possible. This is the
   decision most worth revisiting with fresh eyes -- it is defensible either way, and the reason
   it went this way is that a caller can recover from being told about a dead sandbox and cannot
   recover from never being told.
3. **The liveness probe is a connect and nothing more.** What it proves is exactly one thing and
   the code says so: a process has the Instance's socket bound and accepted a connection on it. It
   is *not* a claim that the guest inside is healthy -- that is what an inspection by exact
   identity answers. A wedged host that accepts and never answers would be reported `live`, and
   that limitation must appear in any evidence document. The connect also clears a socket nothing
   answers on, reusing `channel::connect`, so a host that died leaves no name for a second listing
   to report.

A listing does one connect per Active record. That is a round trip per sandbox and it was chosen
over a cheaper guess deliberately.

## What the next person should do first

1. Rewrite the two failing `soma-api` refusal tests against the new listing document.
2. Get live proof on eval-1, through HTTP and then the CLI. For the terminal, run `tty` in the
   guest and keep the raw answer: `/dev/pts/0` is the proof, `not a tty` is the disproof, and
   nothing else in the run means much without it. Then drive a program that refuses to run without
   one. For `list`, prove all three cases: a sandbox that exists, one that was destroyed and is
   absent from the listing, and one whose host was killed with `SIGKILL` and is reported
   `state: active, host: absent`.
3. Only then touch `docs/claim-ledger.md` and the gap-analysis table.
4. Add the MCP tools.

## Overlap with other agents

The facade (`crates/soma/src/backend/mod.rs`, `crates/soma/src/engine/`) and the surfaces
(`soma-api`, `soma-cli`) are shared with whoever else is working. Inside `soma-local` this touched
`backend/mod.rs`, `backend/kvm.rs`, `backend/kvm/dispatch.rs`, `backend/kvm/host/`,
`backend/kvm/session*`, `backend/kvm/worker*`, `backend/kvm/start.rs` and `file_store/`, which
overlaps the performance and jail work in `crates/soma-local/src/backend/`. One line of
`crates/soma-guest/src/application/pty/codec.rs` was widened to `pub`.
