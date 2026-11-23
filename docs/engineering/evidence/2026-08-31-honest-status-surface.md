# The status and error surface says only what it can back - 2026-08-31

An audit of every place in `crates/soma-cli`, `crates/soma`, `crates/soma-api`, `crates/soma-mcp`,
`benchmarks/local_alpha`, and `scripts` that reports an outcome, branched from `8bdda75`. The
defect class is one thing said four ways: a success that cannot be used, a permanent failure
marked retryable, a discarded error, and a count of failures with no attributable cause.

Every site examined is listed below, including the ones judged correct, so the audit is
reviewable rather than a list of edits.

## What changed

### 1. `soma machine launch` reported a ready identity that died with the command

`crates/soma-cli/src/app/operation.rs`, `crates/soma-local/src/backend/mod.rs`.

The KVM backend holds the machine and its authenticated guest session in the process that
launched it.

**Corrected 2026-09-01.** As written, this sentence also named macOS, and that was wrong: the
macOS adapter holds no machine, and was swept in with KVM by resemblance rather than by
examination. Nothing in this document observes a macOS run. See
[macOS was classified as hosting a machine it does not hold](2026-09-01-macos-hands-back-a-usable-identity.md);
the refusal described below did fire on macOS as this document says, and it should not have.
It was withdrawn on 2026-09-01; the KVM half stood until the machine host closed it.

The command line exits immediately after reporting
`{"state":"ready","instance_id":"..."}`, so the identity it hands back names a machine that is
already gone. `machine exec` from a second process then refused, which is the failure the caller
was told would not happen.

`soma_local::machine_hosting(BackendKind)` now names which backends keep a machine inside the
launching process. Where they do, `machine launch` is refused before a machine is built:

```
{"command":"machine.launch","status":"error",
 "error":{"code":"machine_not_hosted","retryable":false,
          "message":"this backend hosts a machine only inside the launching process, so a
                     launched instance identity would not survive this command; use `soma run`
                     for a single-process sandbox"}}
```
exit 76, `CapabilityUnavailable`. Refusing before the launch is deliberate: a refusal after one
would leave a durable record for a machine nothing owns.

This is backend specific rather than a blanket refusal. The docker backend addresses a container
by a name derived from the Instance, so a launched container does outlive the command; `machine
launch` still succeeds there, proved by the two of two docker burst below. When the KVM machine
runs in a host owned worker, one arm of `machine_hosting` becomes `OutlivesProcess` and launch
succeeds again with no other change.

**Public contract change.** `soma machine launch` on the KVM and macOS backends exits 76 where
it used to exit 0. That is the point, but it is a visible change to anything scripted against it.

### 2. `soma-api` answered 201 for a sandbox no later request could address

`crates/soma-api/src/handler.rs`, `capability.rs`, `facade.rs`, `local.rs`.

The service opens one `LocalRuntime` per connection, by design, so it carries the same defect for
the same reason. `POST /v1/sandboxes` now refuses with the existing `MissingCapability` vocabulary
(`501 capability_unavailable`, `retryable:false`) naming durable machine hosting, before a machine
is built. `SandboxFacade` gained one method, `hosts_addressable_sandboxes`.

**Public contract change.** `POST /v1/sandboxes` answers 501 rather than 201 on KVM and macOS.

### 3. `retryable` promised something no amount of asking could deliver

`crates/soma-cli/src/app/failure.rs`, `crates/soma-api/src/failure.rs`,
`crates/soma-mcp/src/server/failure.rs`.

`retryable` is read by clients as permission to resubmit the identical request. Two conditions
claimed it and could not honour it:

| condition | was | is | why |
| --- | --- | --- | --- |
| `backend_unavailable` (`BackendFailureKind::Unavailable`, `LocalFailureKind::BackendUnavailable`, MCP `Unavailable`) | true | **false** | a capability this host does not have does not appear because the caller asked again; clearing it takes operator action |
| `state_store_failure` from `Corrupt`, `InvalidRecord`, `UnsupportedVersion`, `CapacityExceeded` | true | **false** | none of these clear on their own |
| `state_store_failure` from `Conflict`, `Unavailable` | true | true | a contended lock and a momentarily unreachable store do clear |
| `guest_timeout`, `guest_interrupted`, `backend_failure` | true | true | judged correct; a fresh attempt can succeed |

The state store kinds were being discarded (`ManagedFailure::StateStore(_)`,
`RunFailureKind::StateStore { .. }`) and are now read.

### 4. A prepared entry with no snapshot cold booted in silence

`scripts/prepare-generation.sh`, `scripts/tests/prepare-generation-contract.sh`.

The KVM backend chooses restore over cold boot on the presence of one file,
`<entry>/snapshot/state.somasnap`, and reports neither choice: both are
`PreparationClass::OnDemand` in the receipt. An entry with no snapshot launches, roughly fifteen
times slower, and produces a measurement indistinguishable from a working one.

`prepare-generation.sh` now names the path the entry it just compiled will take, prints the exact
`capture_snapshot` command, and exits 3 on a cold boot only entry.
`SOMA_ALLOW_COLD_BOOT_ENTRY=1` accepts one on purpose. It also prints the shape the entry must be
launched at, because a snapshot restores only the memory it was captured at.

The receipt itself still cannot distinguish a restore from a cold boot. Doing that needs a new
`PreparationClass` set in `crates/soma-local/src/backend/kvm/start.rs`, which this branch is
staying out of.

### 5. A zero scoring burst run said only zero

`benchmarks/local_alpha/burst/attribution.py` (new), `slot.py`, `run.py`, `command.py`,
`validation.py`, `results.py`, `document.py`.

Every `soma` process the harness spawned reported an exit code, and every refusal it printed
carried a typed error code and message. All of it was discarded: `invoke` returned early on a
nonzero exit without reading the envelope, and every failure reason was recorded with an empty
detail.

Now each failure carries an attributable detail (`exit=76(capability_unavailable)
code=machine_not_hosted message=...`), the completion record carries a `failure_breakdown` by
reason, the evidence document leads its failure section with it, and a run that scores below its
attempt count prints it on stderr. `validate_samples` refuses a v2 results file whose failure
names no cause, so the silence cannot come back.

### 6. A relative path refusal was buried, and a delivered shape was never checked

`benchmarks/local_alpha/burst/command.py`, `attribution.py`.

`argparse.error` printed one sentence under a usage block. It is now one line with its own
prefix, naming the path and the absolute form to pass.

Separately, a launch reports `ok` whether each shape dimension was observed to match, observed to
differ, or never checked. Storage is never checked on either the KVM or the docker backend, so a
request for ten gigabytes of writable storage served by a two gigabyte overlay reports `ok`. The
harness now names every such disagreement as a shape mismatch.

The `--storage-mib` default of 10240 turned out to be harmless: measured below, a run at 10240
against a Generation captured at 2048 succeeds. The dimension that actually refuses is
`--memory-mib`, and it refuses as `backend_unavailable`.

### 7. Smaller sites

- `crates/soma-cli/src/main.rs`: a clap refusal discarded which argument failed, leaving stderr
  empty in JSON mode. The argument name and the kind of refusal are now printed. No caller value
  is echoed: only a plain name is reported, and everything after the first space is dropped, so
  the existing "never echo a private image reference" contract is kept.
- `crates/soma-local/src/backend/docker/execute.rs`: a container was recorded as already cleaned
  even when the `docker rm` that was attempted failed, so the later cleanup reported
  `complete_owned_machine` for a container that could still be running. It is recorded only when
  the removal returned success.

## Audited and judged correct

- `crates/soma-api/src/envelope.rs:render` - `unwrap_or_else` on a serialization fault produces a
  valid failure envelope rather than a dropped connection. Documented and correct.
- `crates/soma-api/src/http/server.rs` - `let _ = writer.flush()` after a write that already
  failed. The peer is gone; there is nobody to tell. Documented and correct.
- `crates/soma-cli/src/main.rs` - `let _ = eprintln_bounded(...)` on the output failure path. It
  is the last thing tried before exiting `Software`.
- `crates/soma-local/src/backend/docker/launch.rs` - `let _ = remove(&name)` after a failed
  `docker start`. Best effort, and no later claim depends on it.
- `crates/soma-local/src/file_store/revision.rs` - `let _ = fs::remove_file(&temp)` on a
  temporary that a failed rename left behind. No claim depends on it.
- `crates/soma-cli/src/app/success.rs` - a nonzero guest command carries `guest_nonzero` with
  `retryable:false` and exit 10, and its output is still rendered. Correct.
- `RunFailureKind::CleanupIncomplete`, `ObservationMismatch`, `OutputLimitExceeded` and every
  `ManagedStateError` - already `retryable:false`.
- `scripts/*.sh` uses of `2>/dev/null` - every one found is a `command -v` probe, a
  `git rev-parse` with an explicit `|| echo unknown` fallback, or a diagnostic collector with a
  named fallback. None hides a real failure.
- `benchmarks/local_alpha/burst/results.py:load_results` - already fails closed on an incomplete,
  mixed, or oversized results file.

## Live evidence, eval-1, `d38515b`

Store: `/srv/soma/honest-store`, `busybox:stable-musl` compiled at 1024 MiB and 2048 MiB and
captured at 1024 MiB. All output is verbatim.

**Cold boot only entry is refused (was: silent).** `scripts/prepare-generation.sh busybox:stable-musl
/srv/soma/honest-store /srv/soma/fs-tools 1024 2048` exited **3** with
`boot:   COLD BOOT; this entry has no snapshot` on stdout and, on stderr,
`COLD BOOT ONLY: .../snapshot/state.somasnap does not exist.` naming the exact
`capture_snapshot` command. After capture, the same check prints
`boot:   RESTORE from .../snapshot` and exits 0.

**`machine launch` is refused (was: `ok`, `state: ready`).**

```
$ soma --format json --backend kvm machine launch --instance-id 1111... busybox:stable-musl
{"command":"machine.launch","status":"error","result":null,
 "error":{"code":"machine_not_hosted","retryable":false,"message":"this backend hosts a machine
 only inside the launching process, ... use `soma run` for a single-process sandbox"}}
EXIT=76
```

**A memory shape mismatch, unchanged and still unhelpful.** `soma run --memory-mib 2048` against
a snapshot captured at 1024 exits 76 with `backend_unavailable`, "backend capability is
unavailable". Now correctly `retryable:false`, but the message still does not say shape. The
distinction is made in `crates/soma-local/src/backend/kvm/start.rs`, out of scope for this branch.
`--storage-mib 10240` against the same entry exits **0** and reports `ok`.

**A relative path errors visibly (was: buried under a usage block).**

```
$ python3 -m benchmarks.local_alpha.burst run ... --results relative-results.jsonl -- /bin/busybox true
soma-burst: error: results file path must be absolute, but 'relative-results.jsonl' is relative;
pass /srv/soma/honest/relative-results.jsonl
EXIT=2
```

**A zero scoring run states its reason (was: `0/N` and nothing else).** Eight KVM slots at the
captured shape:

```
soma-burst: 0 of 8 samples succeeded. Why:
  8x cleanup_failed at destroy
      8x exit=66(not_found) code=machine_not_found message=sandbox instance was not found
  8x launch_process_failed at launch
      8x exit=76(capability_unavailable) code=machine_not_hosted message=this backend hosts a
         machine only inside the launching process, so a launched instance identi...
EXIT=1
```

The same breakdown is in the completion record in `/srv/soma/honest-bench/kvm8.jsonl`.

**A successful run that still did not deliver the shape it asked for.** Two docker slots at
`--storage-mib 10240`, scoring two of two:

```
soma-burst: shape mismatch: storage_mib requested 10240, effective not_verified
EXIT=0
```

That run also proves the launch refusal is backend specific: docker hosts a container that
outlives the command, so `machine launch` succeeded for both slots.

## Tests

One test per defect class, failing at `8bdda75` and passing here.

| Test | Proves |
| --- | --- |
| `soma-cli app::failure::tests::an_unavailable_backend_capability_is_not_retryable` | 3 |
| `soma-cli app::failure::tests::an_unavailable_local_backend_is_not_retryable` | 3 |
| `soma-cli app::failure::tests::only_a_clearing_state_store_condition_is_retryable` | 3 |
| `soma-cli app::failure::tests::a_state_store_that_cannot_be_opened_reports_its_own_condition` | 3 |
| `soma-cli app::failure::tests::a_launch_this_process_cannot_host_is_refused_rather_than_reported_ready` | 1 |
| `soma-cli app::failure::tests::the_backends_that_host_a_machine_only_in_this_process_are_named` | 1 |
| `soma-api refusals::creating_a_sandbox_no_later_request_could_address_is_refused_before_it_is_built` | 2 |
| `burst SlotAttributionTests::test_a_refused_launch_keeps_the_reason_the_command_line_printed` | 5 |
| `burst AttributionContractTests::test_a_failure_that_names_no_cause_is_refused` | 5 |
| `burst AttributionContractTests::test_a_completed_run_without_a_breakdown_is_refused` | 5 |
| `burst RelativePathTests::test_the_refusal_names_the_absolute_path_to_pass` | 6 |
| `prepare-generation-contract.sh` boot report cases | 4 |

The three retryable tests were run against a clean `8bdda75` tree with only the test module added
and all three failed there; the four burst tests that exercise changed behaviour failed there
too, four of eight in that file, the other four covering the new helper alone.

## Known interaction

`scripts/reproduce.sh`, added on branch `audit2/final`, calls `prepare-generation.sh` under
`set -e` and captures the snapshot itself on the next line. With this branch's refusal in place
that call aborts before the capture. The one line fix on the merged tree is to invoke it as
`SOMA_ALLOW_COLD_BOOT_ENTRY=1 ./scripts/prepare-generation.sh ...`, which is exactly what the
escape hatch is for: `reproduce.sh` knows it is about to capture. The two guards are
complementary, `reproduce.sh` closing the launch site and `prepare-generation.sh` closing the
primitive that the documented build sequence invokes directly.

## Retained samples

The live runs behind every claim above are retained in
[`raw/2026-08-31-honest-status-surface/`](raw/2026-08-31-honest-status-surface/): the build
manifest the runs were made with, the eight-slot KVM burst that now names why it scored zero
(`kvm8.jsonl`), and the two-slot Docker burst that reports the unverified shape
(`docker2.jsonl`).

The interaction with `reproduce.sh` described above has since been applied on the merged tree:
that script now invokes the primitive with `SOMA_ALLOW_COLD_BOOT_ENTRY=1`, because it captures the
snapshot on the following line and is the one caller entitled to the uncaptured entry.
