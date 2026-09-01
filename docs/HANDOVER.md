# SOMA handover - 2026-08-31

Written so work can continue on another machine, by someone who was not here. It states what is
finished with the evidence that proves it, and what is left with enough detail to start on.

Repository state at writing: `main` at `16d4b34`, 1429 tests passing, clippy zero warnings
workspace wide, `scripts/check-architecture.sh` and `scripts/check-evidence.py` green.

Two branches were in flight when this was written and are listed under **In flight** below. Every
other item is either finished or not started.

---

## Read these first

- [`docs/claim-ledger.md`](claim-ledger.md) is the single place that states what SOMA can do
  today. Its rules are at the top and they are binding: no row may claim more than its evidence
  supports, and a live-proved row names a commit and links a retained artifact.
- [`docs/evidence/2026-08-31-performance-findings.md`](evidence/2026-08-31-performance-findings.md)
  is the consolidated performance table, including what does **not** cost what it looks like.
- [`docs/standards/sota-engineering-standard.md`](standards/sota-engineering-standard.md) carries
  the performance standard and the **mechanism-claim rule** added today. Read that rule before
  asserting why anything is slow.

---

## Finished

Each row links the evidence that proves it. Nothing here is production-admitted.

| Capability | Evidence |
| --- | --- |
| A sandbox that outlives the process that launched it. Five separate `soma` processes drove one sandbox; a file written by the second was read back by the third. The ComputeSDK contract benchmark went from 0 of 100 on every run in this repository's history to **100 of 100 on three independent runs** | [durable machine host](evidence/2026-08-31-durable-machine-host.md) |
| A status surface that reports only what it can back. False `ok` results, permanently fatal errors marked `retryable`, a silent cold boot, and a benchmark scoring zero with no stated reason are all fixed, each with a test that failed before the change | [honest status surface](evidence/2026-08-31-honest-status-surface.md) |
| One command from an OCI image to a measured launch, refusing loudly on every precondition that used to fail silently | [one-command reproduction](evidence/2026-08-31-one-command-reproduction.md), `scripts/reproduce.sh` |
| Evidence integrity enforced in CI: dead links, ledger rows naming commits that do not exist, quoted figures with nothing retained | `scripts/check-evidence.py` |
| The concurrency variance root-caused. The head clone is **serialized, not slow**: 99 of 100 threads park in `xfs_ilock2_io_mmap` while one updates the allocation group refcount btree | [head clone serialization](evidence/2026-08-31-head-clone-serialization.md) |
| The restore resume window explained. It is **EPT violations** on guest first touch, resolved inside KVM without returning to userspace: 3199 of them, ~16.8 ms of in-kernel handling against 11.7 ms of guest execution. Pre-faulting and huge pages were both built and measured, and both made it worse or did nothing | [restore resume page-in](evidence/2026-08-31-restore-resume-page-in.md) |
| The `ready` segment reduced 27.59 to 22.60 ms by removing the readiness probe, confirmed at eight times the memory | [ready segment split](evidence/2026-08-31-eval1-ready-segment-split.md) |
| A Generation that declares no writable storage clones no head. At concurrency 100 its clone is 0.3 ms in all six cohorts against a writable range of 9.0 to 97.6 ms | [device set comparison](evidence/2026-08-31-merged-binary-device-set-c100.md) |
| The Isorun comparison retracted as invalid. Their sample creates an addressable sandbox and commands it by identity; every SOMA figure here was one-shot | [why the pairing is invalid](evidence/2026-08-31-isorun-comparison-is-not-like-for-like.md) |

`soma run` remains the fast one-shot path at **20.5 ms to Ready**, unchanged by any of the above.

---

## In flight, stopped mid-session

Work stopped when the session's budget ran out. **Every branch below is pushed to `origin`**, so
none of it depends on the machine it was written on. Each was told to commit whatever it had, even
non-building, rather than lose it - so assume these branches do not compile until you check.

| Gap | Branch on `origin` | State when stopped |
| --- | --- | --- |
| 1, durable path performance | `perf/durable-path` | 3 commits. Its diagnosis of the 447 ms state write and 966 ms release is the valuable part |
| 2 and 3, PTY and `sandbox.list()` | `feat/pty-and-list` | 2 commits, committed from 55 uncommitted files. Read its own notes first |
| 4, head clone serialization fix | `perf/head-clone` | 3 commits |
| 5, jail split | `feat/jail-split` | 5 commits, committed from 17 uncommitted files |

Two efforts finished and are already merged into `main`: the crowded-file splits, and the EPT
attribution described under gap 7.

**Before resuming any of them**, read the branch's own evidence file. Each was asked to write down
what it had working, what it had not started, and what it learned that the diff does not show.
That note is worth more than the code.

## What is left

Ranked. Each says what it is, why it matters, where the code is, and what would prove it done.

Gap 5 is a security gap and gap 1 is a competitiveness gap. Which comes first is a judgement call
that has not been made.

### 1. The durable path is slow

**1.6 s p50** at concurrency 100, against 20.5 ms for `soma run`. The capability works; the
performance does not. This is the item that decides whether SOMA is competitive, because the
durable path is the one a coding agent uses and the only one comparable to other providers.

The costs are already named and none look fundamental. Under 100 simultaneous callers: the durable
state write is **447 ms** and release is **966 ms**. Machine readiness including the host spawn is
**55 ms p50**, so the machine is not the problem. Receipts say `preparation: on_demand`, meaning
the prepared machine pool - which took machine construction from 3.27 ms to 18.4 us - is not being
used on this path.

Proof: the contract benchmark at a p50 that is a small multiple of `soma run`, not fifty times it.

### 2. PTY is implemented and unreachable

**The guest filesystem half of this gap is closed** - six operations now reach HTTP, the CLI and
MCP on both the resident and hosted paths
([evidence](evidence/2026-08-31-guest-filesystem-surfaces.md)). Follow that pattern for PTY rather
than inventing a second one: it added `Backend::file`, a portable `FileOperation`, a `FileAnswer`
carrying the guest's closed failure set unchanged, and `Engine::file_machine`.

`crates/soma-guest/src/application/pty/` implements a terminal at the protocol level, one session
at a time, and nothing reaches it. A terminal is a stream rather than a request and answer, so it
does not fit the filesystem shape exactly. One recorded hazard: devpts was at one point never
mounted in the guest, which made PTY unreachable in a real guest even though the protocol worked -
verify the guest has what a pty needs before assuming wiring is the only gap.

Note `MAX_FILE_BYTES` is 4 MiB because the hosted relay carries an operation as one JSON line where
a byte becomes up to four characters. A PTY stream hits that constraint harder than a file does.

### 2b. Three defects found while closing the filesystem gap, all fixed

Recorded because each was worse than the gap being closed, and because they suggest where to look
for more of the same. An inadmissible path was rejected by the guest *while decoding*, which is a
protocol fault that ends the session, so a caller sending a relative path destroyed its own sandbox
instead of being told no. `soma-api` never asked for hosted machines, so every sandbox it created
died with the connection and nothing over HTTP had ever worked end to end. And the runtime starts a
host by re-entering `current_exe()` with `machine-host`, which only the command line answered, so
`soma-api` spawned a copy of itself that exited on an unknown option.

### 3. `sandbox.list()` cannot be served

`StateStore` addresses records by exact Instance ID and exposes no enumeration, so the set of live
sandboxes cannot be read back by any process
(`crates/soma-api/src/capability.rs`, `MissingCapability::SandboxEnumeration`). `getById` is closed
by the durable host; `list` is not. It is the last operation missing from the ComputeSDK contract
after gap 2 lands.

### 4. The head clone serialization fix

Root-caused but **not shipped**, deliberately: neither half is reachable through an existing seam.
Two serialized kernel objects behave as an AND gate, and fixing either alone relocates the cost
rather than removing it - throughput moves about 6 percent. Both together measured 234.7 to
**159.3 us per clone**, worst cohort 27.85 to 17.65 ms.

The two halves are sharding the head directory (small, behind the existing `SOMA_HEAD_DIR` seam)
and fanning the overlay template into independent physical extents (a Generation artifact change,
2 GiB each). The alternative worth weighing first is pre-cloning heads off the launch path
entirely; the read-only arm shows that ceiling.

One honest gap remains in that investigation: what puts a *particular* cohort into the slow mode on
an idle host is not identified. The probe alone spreads 1.98x over 150 cohorts while the real
launcher spreads 9.88x, and the difference is the 100 machines each restoring a 1 GiB snapshot.
Stated as a hypothesis in the record, not a finding.

### 5. The machine host cannot be jailed as it is, because it is a server

Investigated and **not built**, deliberately. The result is a measured negative that changes the
shape of the work rather than just deferring it.
[Evidence](evidence/2026-08-31-jailing-the-machine-host.md).

The blocker the previous record predicted - needing `openat` throughout - is not the real one.
`socket`, `bind`, `listen` and `accept4` are in `soma_jail::NEVER_ALLOWED`, and the test on the
compiled BPF proves `KILL_PROCESS` for each in **both** jail phases. **A jailed process cannot be a
server.** That is deliberate, not an oversight. A full `strace -ff` of the host across a lifecycle,
classified against the real tables, found 15 syscalls killed always and 11 more admitted only in
the startup phase. Notably `readlink`: the overlay head recovers its path through
`/proc/self/fd/<n>` (`boot.rs:221`), and the jail refuses service unless procfs is invisible.

The constructive half is the useful part. **All 27 KVM ioctls the machine issues are already on the
jail's allowlist**; the only two missing belong to head creation. The jail was designed correctly
for the machine and refuses only the hosting. So the shape that fits is a **split, not a move**: an
unjailed broker keeps the socket and the filesystem and hands a sealed descriptor table to a jailed
machine, which speaks over the pre-connected `Control` descriptor `soma-vmm` already declares.

That port was not started because it is a real multi-session change: `soma-vmm`'s `Platform` is
`UnavailablePlatform`, a stub that restores nothing, and reaching it means roughly 115 path
operations across five crates becoming descriptor operations. Until it is done, the host holding a
live KVM machine runs unconfined, and that is the largest security gap in the system.

### 6. macOS hands back an identity that cannot be used - CLOSED 2026-09-01, and it was not the gap it looked like

The premise was wrong. The macOS adapter holds no machine: it requires Apple's runtime service to
be running, registers each machine as `soma-<instance_id>` with an ownership label carrying the
same identity, and re-proves ownership from that service's own record before every operation. It
is the Docker shape, down to the container name. Even one launch already survives two process
deaths, because create, start, and inspect are three separate `container` processes.
`machine_hosting` was corrected rather than a second machine host built, which would have held
nothing and duplicated a service the probe already requires. See
[macOS hands back a usable identity](evidence/2026-09-01-macos-hands-back-a-usable-identity.md).

**Still open, and it needs a Mac.** The correction rests on reading code and on a component test
that runs on Linux; no macOS host is reachable from this repository's test host, and the backend
does not compile off macOS. The run that would settle it is the five-process lifecycle the KVM
machine host records. Two smaller things a Mac would also settle are named at the end of that
document.

### 7. EPT violations are the largest remaining cost in `ready`

About 16.8 ms of in-kernel exit handling, proportional to the number of distinct guest pages the
resume touches **and to nothing else**. Pre-faulting the whole image moves nothing and costs 57 ms;
huge pages are worse because a `MAP_PRIVATE` mapping cannot hold a huge EPT entry. Both were built
and measured. The only lever is touching fewer guest pages on resume, which is snapshot and
kernel-state work that has not been attempted.

### 8. Loopback-only network repair on a declined egress - CLOSED 2026-09-01, mostly before this entry was written

It was never a launch-page wire change. `ba0cde7` had already made a Generation declaring the
isolated policy a machine with **no network device**, and the guest reads that from its own
command line and calls `repair_loopback_only`. So the answer is carried by the absence of a
device rather than by a value on the page, and `LaunchNetwork` is untouched. What was missing was
the proof and the written decision, and both now exist:
[a sandbox with no egress still reaches itself](evidence/2026-09-01-loopback-only-repair.md) is
ten of ten sandboxes holding only `lo`, printing no routes at all, and binding and connecting to
`127.0.0.1`; [ADR 0040](adr/0040-no-egress-is-the-absence-of-a-device.md) records the design,
including the deliberate decision to stop writing `/etc/hosts` on this path.

**One residual, unreachable today.** The device set tests the declared policy *class* while the
launcher tests the request's `EgressPolicy`, and for `RuntimeDefault` they disagree: such a
Generation is built with a network device it can never use and pays the full repair to install a
gateway and resolver of `10.0.0.1` that route nowhere. No command can build one, because every
tool that prepares a Generation compiles the isolated policy. The ADR says why the obvious patch
was rejected and where the correct fix belongs. The cost of the repair step was also not
re-measured: the guest's per-step timing needs the `timing-report` agent build and its console,
which no command-line path surfaces.

### 9. Eight files sit at exactly 300 lines

The architecture limit is being treated as a target rather than a ceiling, so the next honest error
message in any of them breaks the build and the cheapest fix is always deleting an explanation.
`scripts/reproduce.sh` was split today along a real seam as the pattern to follow. The rule: split,
or ask for an exemption - never delete the reason something exists.

### 10. Then re-measure against Isorun, honestly

Only once gap 1 is closed. The comparable figure is the durable path measured create-then-command-
by-identity, matching their flow. It will be worse than the one-shot numbers in this repository,
and it will be the first one that means anything. The engineering standard now forbids printing the
old pairing.

---

## Environment

**eval-1** is the test host. `ssh eval1` works with no ssh-agent; the key is in `~/.ssh/config`.
80 logical CPUs, XFS with reflink at `/srv`, 1.4 TB free. `source ~/.cargo/env` first - cargo is
not on the non-interactive PATH.

Use `scripts/reproduce.sh` rather than the seven-step build sequence. It refuses loudly on every
precondition below.

**Traps that each cost hours today:**

- A prepared entry with **no captured snapshot does not fail**. It cold boots, roughly fifteen
  times slower, and reads exactly like a working measurement. Compiling a Generation and capturing
  one are two commands and only the first is in `prepare-generation.sh`.
- The shape at launch must **exactly** match the shape the Generation was captured at, or every
  launch is refused before a machine exists and the harness reports zeroes.
- `kernel/out` is a symlink to a built kernel and is deliberately untracked. It re-entered git
  through a merge once because `.gitignore` had a trailing slash, which matches a directory but not
  a symlink.
- Prepared stores go stale when the wire contract changes and cannot launch. `/srv/soma/sweep` and
  `/srv/soma/prepared` are stale examples.
- `/srv/soma/*` is **not safe shared space**. One agent deleted another's 38 GB of samples there
  today. Use your own directory and clean it up.
- Host load contaminates any timing measurement, and the one-minute load average is unusable as a
  gate because a 100-sandbox cohort raises it by itself. Sample `/proc/stat` while your own run is
  idle instead.

---

## Working rules

- **Measure before asserting a mechanism.** Roughly ten reasoned hypotheses were contradicted by
  measurement in one session, and three were only settled by building the change and measuring it
  worse than what it replaced. The rule is in the engineering standard.
- **One cohort is not a distribution.** A single pair of cohorts ranked two configurations
  backwards; six per arm reversed it, and forty per arm showed the spread was 9.88x rather than the
  3.2x six had suggested.
- **A negative result is a result.** Retain it, so nobody pays for the same hypothesis twice.
- Never put `Co-Authored-By` or `Claude-Session` trailers in a commit message.
- Simple over engineered: build only what is optimal, with narrow public surfaces and no
  speculative seams.
- If agents run in parallel, give each its own git worktree. Four agents shared one checkout today;
  one agent's `git clean` deleted another's files mid-build and a third had its commits swept into
  a peer's branch.
- Gates before claiming anything done: `cargo fmt --all --check`, clippy zero warnings workspace
  wide, `cargo test --workspace`, `scripts/check-architecture.sh`, `scripts/check-evidence.py`,
  `typos`.
