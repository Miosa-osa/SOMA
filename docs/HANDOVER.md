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

## In flight

Both were running when this was written. Check whether their branches merged before starting either.

1. **Jail the durable machine host** - branch `feat/jail-machine-host`. The host currently runs as
   an ordinary unjailed child process holding a live KVM machine, while everything else in the
   system runs confined. The blocker is known: a jailed process has no usable filesystem
   namespace, so the KVM lifecycle needs directory descriptors and `openat` throughout instead of
   paths.
2. **Guest filesystem through the facade** - branch `feat/guest-filesystem`. See gap 2 below for
   what it is closing.

---

## What is left

Ranked. Each says what it is, why it matters, where the code is, and what would prove it done.

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

### 2. Guest filesystem and PTY are implemented and unreachable

`crates/soma-guest/src/application/filesystem/` implements six operations and
`crates/soma-guest/src/application/pty/` implements a terminal. The portable facade's `Backend`
trait (`crates/soma/src/backend/mod.rs:24`) carries only `kind`, `resolve`, `launch`, `execute`,
`inspect` and `cleanup`, so no engine call reaches either. `crates/soma-api/src/route.rs` already
routes `/v1/sandboxes/{instance}/filesystem/{operation}` and returns a capability refusal.

Filesystem is in flight. **PTY is not started.** Both are wiring, not invention.

Proof: through the HTTP API, because that is what a ComputeSDK client calls. Include a binary-safe
round trip - non-UTF-8 bytes in, identical bytes out.

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

### 5. macOS hands back an identity that cannot be used

`machine_hosting(BackendKind::MacosVirtualization)` is `LaunchingProcess`
(`crates/soma-local/src/backend/mod.rs`). It is now the only backend that does. The CLI and the
HTTP API both refuse a launch there rather than lying about it, which is correct but is a refusal,
not a capability. Closing it means the same host-process work done for KVM.

### 6. EPT violations are the largest remaining cost in `ready`

About 16.8 ms of in-kernel exit handling, proportional to the number of distinct guest pages the
resume touches **and to nothing else**. Pre-faulting the whole image moves nothing and costs 57 ms;
huge pages are worse because a `MAP_PRIVATE` mapping cannot hold a huge EPT entry. Both were built
and measured. The only lever is touching fewer guest pages on resume, which is snapshot and
kernel-state work that has not been attempted.

### 7. Loopback-only network repair on a declined egress

Worth **at most** 2.65 ms and probably less. `network_repair::repair` installs unroutable egress
values on a sandbox that was given no network, but the same function also raises loopback, which a
fresh guest leaves down - skipping it wholesale would break anything binding `127.0.0.1`. Blocked
on a design decision: `LaunchNetwork` (`crates/soma-guest/src/launch_page/network.rs:48`) has no
representation for "no egress", so this is a launch-page wire change. Low priority, recorded so
nobody re-derives it.

### 8. Eight files sit at exactly 300 lines

The architecture limit is being treated as a target rather than a ceiling, so the next honest error
message in any of them breaks the build and the cheapest fix is always deleting an explanation.
`scripts/reproduce.sh` was split today along a real seam as the pattern to follow. The rule: split,
or ask for an exemption - never delete the reason something exists.

### 9. Then re-measure against Isorun, honestly

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
