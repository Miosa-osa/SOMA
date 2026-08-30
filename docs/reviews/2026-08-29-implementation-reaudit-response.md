# Implementation re-audit response

- Date: 2026-08-29
- Repository: `Miosa-osa/SOMA`
- Responds to: [the implementation re-audit](2026-08-29-implementation-reaudit.md)
- Re-audited implementation commit: `d790555`
- Response branch: `fix/audit2`, rebased on `origin/main`
- Response status: seven of nine findings closed in code with regression tests, one closed on `main`, one open

This document maps every re-audit finding to the commit that fixed it and to the regression test that proves each acceptance gate.
It also states plainly which findings remain open and why.

Commit identifiers below are the identifiers on the rebased `fix/audit2` branch.
Finding P1.4 is the single exception: it was fixed on `main` before this branch existed, and its commit identifier is a `main` identifier.

## Summary

| Finding | Status | Fixing commit | Proving test |
|---|---|---|---|
| P0.1 Bind network activation to authenticated guest evidence | Mechanism fixed, claim corrected; the residual gap is stated rather than closed | `b7fea48`, scoped by `b09a7ad` and `e45d926` | `forwarding_stays_off_for_every_unauthorized_activation`, `the_challenge_holder_alone_mints_an_accepted_receipt` |
| P0.2 Authorize the privileged `soma-netd` control socket | Fixed | `f008e29`, `ee42a16`, `7455059`, `4465c18` | `the_control_socket_grants_each_operation_only_to_its_capability`, `an_unadmitted_peer_is_closed_before_it_can_send_or_receive_anything` |
| P0.3 Resolve the duplicate and incompatible ADR 0024 decisions | Fixed | `a99dcad` | `scripts/check-architecture.sh` duplicate-ADR-number gate |
| P1.1 Require authenticated readiness evidence after restore | Fixed as a recorded transition; it gates nothing yet, and the ledger says so | `85ae483`, `aeb8a50`, scoped by `c91f17e` | `a_receipt_authenticates_only_against_its_own_challenge_and_identity`, `a_receipt_naming_a_session_the_published_page_does_not_bind_is_refused` |
| P1.2 Bound and supervise privileged networking tools | Fixed | `418c584`, `3b7a8ab`, `7f651ef` | `a_tool_that_ignores_the_polite_signal_cannot_outlive_its_deadline`, `a_tool_that_floods_its_output_is_terminated_rather_than_buffered` |
| P1.3 Require complete network protocol delivery | Fixed | `4ed4e17`, `75356fe`, `2f2933d`, `3fa1226` | `a_complete_reply_reaches_the_peer_exactly_once`, `one_operation_holds_one_assignment_however_often_it_is_replayed` |
| P1.4 Repair the portable benchmark test gate | Fixed on `main` | `e7a076a` | `./scripts/check.sh portable` benchmark stage, which now discovers and runs all five burst modules |
| P1.5 Regenerate snapshot evidence for the current authority design | Fixed | Documentation corrected in `a99dcad`, `1fe154d`, `314bc22`; recapture run at `5d71524` | The six live `x86_64_snapshot_restore` tests, retained in [the current-authority evidence](../evidence/2026-08-30-x86_64-snapshot-restore-current-authority.md) |
| P1.6 Reconcile contradictory decision-map and guide status | Fixed | `1fe154d`, `906d664`, `b498715`, `314bc22`, `6ed3a80`, `de92117`, `f2a2cff` | `scripts/check-architecture.sh` status-vocabulary and claim-ledger gates |

## P0.1 Bind network activation to authenticated guest evidence

Fixed in `b7fea48`, "fix(netd): require an authenticated guest activation receipt before forwarding".

`RepairAttestation::authenticated` is gone.
The broker now samples a fresh single-use `ActivationChallenge` while it assigns a bundle and returns it only to the peer that claimed that assignment.
`crates/soma-guest/src/activation.rs` mints an `ActivationReceipt` as a keyed tag over an `ActivationScope` that binds the Instance, assignment generation, Launch operation and admitted network intent, together with the live Noise transcript.
`Activate` is refused unless the receipt verifies against the challenge the broker itself issued, and the challenge is consumed exactly once.

Acceptance gates and the tests that prove them:

| Gate | Test |
|---|---|
| A raw `Activate` request without an authenticated receipt cannot enable forwarding | `forwarding_stays_off_for_every_unauthorized_activation` in `crates/soma-netd/tests/live_linux.rs` |
| A receipt from another Instance, generation, operation, or network intent is rejected | `a_receipt_does_not_authenticate_for_another_assignment_or_challenge` in `crates/soma-guest/tests/network_activation.rs`; `scopes_reject_every_zero_field` and `verification_binds_the_challenge_scope_and_transcript` in `crates/soma-guest/src/activation/tests.rs` |
| A replayed receipt is rejected | `a_second_session_cannot_reuse_the_first_transcript_binding` in `crates/soma-guest/tests/network_activation.rs`; replay of an already-activated assignment is answered from the receipt that activated it, proved by `sterile_bundle_stays_down_until_activation_and_policy_holds_after_it` in `crates/soma-netd/tests/live_linux.rs` after `2f2933d` |
| Forwarding remains disabled before the authenticated transition and after failed activation | `assert_sterile` and `assert_forwarding_off` in `crates/soma-netd/tests/live/checks.rs`, driven by both live tests above |

### The residual gap, stated plainly

The capability is unforgeable by a third party, but it is not guest evidence.
The challenge is the only secret in the scheme, and the broker generates it and hands it to the claiming peer in cleartext.
Any holder of that challenge can therefore compute an accepted receipt with no guest session, no handshake and no repair.
What the receipt proves is exactly claimant continuity plus single use.

Rather than let the type name carry an authority the value does not have, `b09a7ad` documents the property in `crates/soma-guest/src/activation.rs`, corrects the crate summary and the claim ledger, and pins the property with a deliberately named gate: `the_challenge_holder_alone_mints_an_accepted_receipt` mints an accepted receipt from an invented transcript and no session at all.
`e45d926` makes the same correction for the session transcript, which attests identity rather than repair.

Closing the remaining gap needs a secret the presenter does not hold, meaning a guest-held key the broker can verify against.
That key does not exist yet, and no row in the claim ledger claims it does.

## P0.2 Authorize the privileged `soma-netd` control socket

Fixed in `f008e29`, "fix(netd): authorize the privileged control socket by peer identity and capability", with three follow-ups.

`crates/soma-netd/src/listener.rs` places the listener in an explicitly owned directory, sets and verifies socket owner, group and mode after binding, and fails closed on any drift.
`crates/soma-netd/src/authority.rs` maps each operation to the capability it requires and admits a connection only on a kernel-derived `SO_PEERCRED` identity holding that capability.
`ee42a16` bounds the receive side of every accepted connection so a silent admitted peer cannot wedge the single-threaded broker.
`7455059` proves the socket directory before changing its owner or mode.
`4465c18` binds a release to the peer recorded in the durable assignment, so an admitted peer cannot release another peer's bundle.

Acceptance gates and the tests that prove them:

| Gate | Test |
|---|---|
| An unauthorized local process cannot connect successfully or obtain a descriptor | `an_unadmitted_peer_is_closed_before_it_can_send_or_receive_anything` in `crates/soma-netd/src/listener/tests.rs` |
| A permitted process without the correct operation capability cannot claim, activate, release, or reconcile | `the_control_socket_grants_each_operation_only_to_its_capability` in `crates/soma-netd/tests/live_linux.rs`; `every_operation_names_the_capability_it_requires` and `each_capability_is_granted_separately_and_unknown_peers_are_refused` in `crates/soma-netd/src/authority/tests.rs` |
| Restart, stale-socket, ownership-drift, and permission-drift tests fail closed | `a_restart_replaces_its_own_socket_and_refuses_any_other_stale_path`, `the_socket_and_its_directory_are_owned_and_fail_closed_on_drift`, and `ownership_and_mode_decisions_refuse_every_drifted_node` in `crates/soma-netd/src/listener/tests.rs` |
| Transferred descriptors and replies are bound to the authenticated request identity | `an_admitted_peer_carries_its_kernel_derived_identity` in `crates/soma-netd/src/listener/tests.rs`; peer-bound release proved by `hundred_way` in `crates/soma-netd/tests/live/burst.rs` |
| An admitted peer cannot wedge the broker | `an_admitted_peer_that_stays_silent_loses_its_connection_instead_of_wedging_the_broker` in `crates/soma-netd/src/listener/tests.rs` |

The final gate of this finding, that the production launcher proves the exact identity and capability handoff used by `soma-hostd` and the jailed VMM, is not met.
That handoff does not exist yet; it is part of Ticket 12 and is tracked as a capability gap, not as an unfixed defect.

## P0.3 Resolve the duplicate and incompatible ADR 0024 decisions

Fixed in `a99dcad`, "docs(adr): give the duplicate 0024 decisions unique identifiers".

`0024-pre-launch-snapshot-capture-point.md` is renumbered to `0030-pre-launch-snapshot-capture-point.md`, its Generation-scoped responder-key provisions are marked superseded with a direct link to `0024-per-instance-guest-responder-authority.md`, and every ambiguous `ADR 0024` reference across the ADRs, guides, research documents, evidence and source comments is repointed at the exact active decision.
The repository-wide sweep for the obsolete Generation-scoped secret model is recorded in the same commit, and `f2a2cff` removes the last remaining description of it from the Template guide.

The gate is mechanical: `scripts/check-architecture.sh` gained a check in `a99dcad` that fails when two ADR files share a number, so the defect cannot recur silently.

## P1.1 Require authenticated readiness evidence after restore

Fixed in `85ae483`, "fix(soma-kvm): require authenticated readiness evidence after restore", tightened by `aeb8a50`.

`Restored::ready` no longer accepts an assertion.
It consumes a `ReadinessReceipt` minted from that restore's own single-use challenge and bound to the fresh Instance identity, the exact restored snapshot, the published launch authority, the Launch operation, and one authenticated repaired guest session.
`aeb8a50` additionally requires the receipt to name the session the published launch page binds, closing the case where a receipt named some other authenticated session.

Acceptance gates and the tests that prove them, in `crates/soma-kvm/src/snapshot/readiness/tests.rs` unless noted:

| Gate | Test |
|---|---|
| A caller cannot advance the typestate by assertion | `a_receipt_authenticates_only_against_its_own_challenge_and_identity` |
| Every bound field is load-bearing | `every_bound_session_field_changes_the_receipt` |
| An unbound session or empty challenge is refused | `an_unbound_session_and_an_empty_challenge_are_refused` |
| The receipt must name the session the published page binds | `a_receipt_naming_a_session_the_published_page_does_not_bind_is_refused` |
| No secret bytes leak through diagnostics | `neither_the_challenge_nor_the_receipt_prints_its_bytes` |
| The published page layout matches what the guest crate builds | `the_published_page_offsets_match_the_launch_page_the_guest_crate_builds` |
| The restore path exercises the transition end to end | the `run` and `drive` paths in `crates/soma-kvm/tests/x86_64_snapshot_restore/instance.rs` |

The third part of the required correction, keeping execution and network activation unavailable until the transition succeeds, is **not** satisfied, and `c91f17e` says so in the module documentation and in the claim ledger.
No execution or network-activation seam consumes the transition today, so a refused or never-attempted readiness records a fact rather than withholding anything.
The gate becomes real when the integrated Host path exists; that is Ticket 12, a capability gap.

## P1.2 Bound and supervise privileged networking tools

Fixed in `418c584`, "fix(netd): contain the privileged nft and conntrack invocations", with two follow-ups.

The repository's supervision primitive was extracted out of `soma-generation` into a new `soma-supervise` crate holding the process-group, absolute-deadline and bounded-capture machinery, and `crates/soma-netd/src/nft.rs` now runs `nft` and `conntrack` through it.
Input write failure is an operation failure rather than a discarded result.
`3b7a8ab` forces the whole group when its leader exits, so a descendant holding the pipes cannot outlive the invocation.
`7f651ef` replaces the unbounded namespace listing with a single bounded question about one table's presence.

Acceptance gates and the tests that prove them:

| Gate | Test |
|---|---|
| A tool cannot outlive its absolute deadline | `a_tool_that_ignores_the_polite_signal_cannot_outlive_its_deadline` in `crates/soma-netd/src/nft/tests.rs`; `a_tool_that_ignores_the_polite_signal_is_forced_after_the_grace` in `crates/soma-supervise/src/contained/tests.rs` |
| Capture is bounded and overflow terminates the group | `a_tool_that_floods_its_output_is_terminated_rather_than_buffered` in `crates/soma-netd/src/nft/tests.rs`; `a_stream_that_exceeds_the_capture_ceiling_terminates_the_group` in `crates/soma-supervise/src/contained/tests.rs` |
| The complete process group is terminated, not just the leader | `a_descendant_holding_the_pipes_cannot_wedge_the_broker` in `crates/soma-netd/src/nft/tests.rs`; `a_descendant_holding_both_pipes_cannot_outlive_the_invocation` and `a_descendant_holding_the_input_pipe_cannot_block_the_feed` in `crates/soma-supervise/src/contained/tests.rs` |
| Ruleset input write failure is the operation's failure | `a_ruleset_the_tool_refuses_to_read_is_the_operations_failure` in `crates/soma-netd/src/nft/tests.rs`; `an_input_write_to_a_tool_that_never_reads_is_a_caller_failure` and `a_feed_failure_terminates_the_group_and_returns_the_caller_error` in `crates/soma-supervise/src/contained/tests.rs` |
| Status and parsed output stay available under the bound | `a_bounded_tool_reports_its_status_and_the_output_the_parsers_read` and `one_tables_presence_is_decided_from_its_own_bounded_output` in `crates/soma-netd/src/nft/tests.rs` |
| An absent or unstartable tool is typed, not a panic | `an_absent_tool_is_a_typed_failure_rather_than_a_panic` in `crates/soma-netd/src/nft/tests.rs`; `a_program_that_cannot_be_started_is_a_spawn_failure` in `crates/soma-supervise/src/contained/tests.rs` |
| Cleanup stays recoverable when a step fails | `a_failing_step_does_not_abandon_the_steps_after_it` in `crates/soma-netd/src/release/tests.rs`, added by `3fa1226` |

## P1.3 Require complete network protocol delivery

Fixed in `4ed4e17`, "fix(netd): require complete delivery of every broker reply", with three follow-ups.

`crates/soma-netd/src/daemon/delivery.rs` requires `sent == bytes.len()` and treats any short send as a terminal protocol failure.
The lifecycle rule is now explicit: the mutation commits before reply delivery, and recovery is made idempotent through operation identities plus ledger reconciliation.
`75356fe` keeps the operation identity reserved until its release completes and `2176952` frees it when a ledger-record release completes, so a disconnect during release cannot strand or double-issue an identity.
`2f2933d` answers a replayed activation from the receipt that already activated.

Acceptance gates and the tests that prove them:

| Gate | Test |
|---|---|
| A complete reply reaches the peer exactly once | `a_complete_reply_reaches_the_peer_exactly_once` in `crates/soma-netd/src/daemon/delivery/tests.rs` |
| A short or impossible send is terminal | `a_reply_to_a_departed_peer_is_a_terminal_protocol_failure` in `crates/soma-netd/src/daemon/delivery/tests.rs` |
| A stalled reader is disconnected, not served forever | `a_peer_that_stops_reading_refuses_delivery_rather_than_blocking_the_broker` in `crates/soma-netd/src/daemon/delivery/tests.rs`; `a_peer_that_stops_reading_its_replies_is_disconnected_rather_than_served_forever` in `crates/soma-netd/tests/live/delivery.rs` |
| Uncertain descriptor-transfer delivery leaves no bundle and replays cleanly | `a_claim_the_peer_cannot_receive_leaves_no_bundle_and_replays_cleanly` in `crates/soma-netd/tests/live/delivery.rs` |
| Replay is idempotent through the operation identity | `one_operation_holds_one_assignment_however_often_it_is_replayed` in `crates/soma-netd/tests/live/delivery.rs` |
| Resource pressure across a hundred concurrent lifecycles holds all of the above | `hundred_way_prepare_assign_activate_release_burst` in `crates/soma-netd/tests/live_linux.rs` |

## P1.4 Repair the portable benchmark test gate

Fixed on `main` in `e7a076a`, "test(benchmarks): import burst fixtures the way the repository gate discovers them".

All five burst modules, `test_burst_plan.py`, `test_burst_report.py`, `test_burst_results.py`, `test_burst_run.py` and `test_burst_slot.py`, now import `benchmarks.tests.burst_fixtures` absolutely, which is the import model the discovery command in `./scripts/check.sh portable` actually uses.
No test was removed and discovery was not weakened; only the import form changed.

The proving gate is `./scripts/check.sh portable` itself: its benchmark stage discovers and runs all five modules instead of failing with `ImportError: attempted relative import with no known parent package`.
This branch is rebased on `origin/main`, so the fix is present here.

## P1.5 Regenerate snapshot evidence for the current authority design

**Closed by measurement, not by a commit.**

Closing this finding was a measurement rather than a code change, and the measurement has now been made.
The run is retained as [x86_64 capture and restore on the per-Instance authority design](../evidence/2026-08-30-x86_64-snapshot-restore-current-authority.md): commit `5d71524`, the pinned kernel and static agent by digest, the Generation and all three snapshot objects by digest, on a KVM host, with six live tests passing.

It establishes what the finding asked for. No Instance responder identity appears in `memory.raw`, `overlay.raw`, or `state.somasnap`, and no occurrence of the launch-page domain in guest RAM decodes as a launch page; the launch page slot at `0xd0100000` lies outside the `[0, 0x40000000)` range the memory object covers, so capture at the disconnected repair point cannot carry launch material. A tampered manifest, a tampered memory object, and a foreign CPU template are each refused before any vCPU exists.

What this branch did instead was stop the stale artifact from certifying current bytes:

- `docs/evidence/2026-08-29-x86_64-snapshot-restore.md` now opens with `## Status: historical`, names its run commit `7c1127d`, states that the Generation-scoped responder private key it scanned was removed by ADR 0024, and points at this finding for the recapture.
- The claim ledger carried two separate rows while the only run was historical. The recapture at `5d71524` made them one capability again, so they are now a single live-proved row that still links the `7c1127d` artifact as historical.
- The original observations are retained exactly as recorded rather than rewritten.


## P1.6 Reconcile contradictory decision-map and guide status

Fixed in `1fe154d`, "docs: adopt one status vocabulary and add a claim ledger", with six follow-ups.

`docs/standards/sota-engineering-standard.md` now defines exactly five status terms: designed, component-tested, live-proved, integrated, production-admitted.
`docs/claim-ledger.md` is the single place that states what SOMA can do today; every row uses one of those terms, names the commit for every live-proved statement, and links the retained artifact.
The decision map, module map, how-it-works guide, Generation research, Template guide and README were updated in the same commit, so Ticket 7 and Ticket 8 no longer contradict each other.

The follow-ups finish the sweep: `906d664` names the commit whose pinned config matches the certified kernel, `b498715` carries the historical marker the device evidence already stated, `314bc22` marks the two cold-boot rows historical and names their commits, `6ed3a80` states the commit rule the retained artifacts actually satisfy, `de92117` records the tool-containment and reply-delivery capabilities, and `f2a2cff` drops the Generation responder key from the Template guide.

The gate is mechanical: `scripts/check-architecture.sh` gained checks in `1fe154d` that fail when a status word outside the vocabulary is used and when the ledger drifts from the standard, so the vocabulary cannot silently fork again.

## What remains open, and why

There are exactly two categories of remaining work, and they are different in kind.

### One finding is genuinely open

**P1.5** is closed by the recapture at `5d71524`, retained in [the current-authority evidence](../evidence/2026-08-30-x86_64-snapshot-restore-current-authority.md).
No amount of code or documentation can close it; the artifact has to be produced.
Until it is, the retained snapshot evidence is labeled historical and the claim ledger records the current design as component-tested.

Two closed findings also carry a stated residual that the reader should not mistake for a passing gate:

- **P0.1**: the activation capability proves claimant continuity and single use, not guest repair. The residual is pinned by a named test and stated in the module documentation and the claim ledger. Closing it needs a guest-held key the broker can verify against, which does not exist yet.
- **P1.1**: the readiness receipt records the restored ready transition but gates nothing, because no execution or network-activation seam consumes it. The module documentation and the ledger row both say so.

### The incomplete production gates are capability gaps, not defects

The re-audit's own "Incomplete production gates" section is a list of things SOMA has not built yet.
None of them is a defect in the code that exists, and none of them is closable by a fix commit:

- Ticket 5 and Phase 2, real guest networking: virtio-net attach, TAP transfer, proxy and ingress are designed, not implemented.
- Ticket 6 and Phase 5, Generation certification: `certify_candidate` fails closed as unimplemented; no signed manifest, SBOM, revocation or registry pipeline exists.
- Ticket 9 and Phase 4, real VMM jail: the retained proof constrains `jail-probe`, not the real `soma-vmm` process.
- Ticket 10, production connectivity: proxy attachment, ingress forwarding, jailed TAP transfer and VMM virtio-net integration remain open. Daemon authorization, the one part of Ticket 10 that was a defect, is closed by P0.2.
- Ticket 11, production writable storage: the launch path does not yet consume prepared private overlay heads.
- Ticket 12, Host composition: `soma-hostd` starts only with the explicitly requested development launcher.
- Ticket 13, public KVM Backend: `crates/soma-local/src/backend/kvm.rs` answers every lifecycle call with a typed unavailable failure.
- Ticket 14 and Phase 6, accepted performance evidence: no KVM cohort, signed report, 100 engineering bursts, 10,000 samples, or accepted 10 ms result exists.

Every one of these is a row in the claim ledger with status designed or component-tested.
The correct public statement is unchanged from the re-audit's own conclusion: SOMA has strong component foundations and live KVM proofs, but not yet one production-integrated sandbox lifecycle or an admitted 10 ms performance result.

## Required repair order, as executed

| Step | Status |
|---|---|
| 1. Fix the broken portable benchmark test gate | Done on `main` in `e7a076a` |
| 2. Remove forgeable network activation and authorize the privileged socket | Done in `b7fea48` and `f008e29`, with the P0.1 residual stated |
| 3. Add deadline-bounded tool supervision and unambiguous delivery semantics | Done in `418c584` and `4ed4e17` |
| 4. Bind restore readiness to authenticated guest evidence | Done in `85ae483` and `aeb8a50`; the transition is recorded, not yet consumed |
| 5. Resolve duplicate ADR numbering and obsolete responder-key documentation | Done in `a99dcad` |
| 6. Rerun snapshot evidence on the current authority design | Done at `5d71524`; see [the current-authority evidence](../evidence/2026-08-30-x86_64-snapshot-restore-current-authority.md) |
| 7 through 10 | Not started; these are the capability gaps above, and step 6 gates them |
