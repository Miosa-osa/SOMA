# Guest capabilities proved against a real KVM guest - 2026-08-31

## Evidence boundary

This run proves that four capabilities of the guest control protocol work against a real
`node:22` guest running on real KVM, on the same restore path the snapshot proofs use: the
bounded filesystem operations, the command context, the interactive terminal, and in-guest
secret delivery. Before it, each of them had unit and component tests only, and every one of
those tests ran the guest side of the protocol in the host's own process against a loopback
transport.

Proved live by this run:

- A file the host wrote over the authenticated session is read by a process inside the guest,
  and a file that process wrote comes back to the host byte for byte.
- A payload larger than one record round trips through the chunking helpers unchanged, and the
  guest's own kernel agrees about its length.
- A directory larger than one listing pages, and the pages partition it: every entry is seen,
  and none is seen twice.
- Four filesystem refusals reach the caller as the cause that actually happened rather than as
  the catch-all: an absent path, a directory read as a file, a non-empty directory removed
  without consent, and a path created where one already exists. The same directory is then
  removed once it is empty, which is what says the refusal was about the contents.
- A command sees the environment variable, the working directory, and the account it was given,
  reads the standard input it was handed, and still finds the agent's base `PATH`.
- The terminal is a real pseudo-terminal: it echoes what is typed at it, reports its own
  dimensions through the guest kernel, reports the new ones after a resize, reports the end of
  the session once its shell exits, and refuses a write afterwards.
- A delivered secret exists in the guest at exactly the requested mode with exactly the value's
  bytes, and the value appears in none of the guest console, the session digest the readiness
  receipt is minted over, the rendered placement, or the three snapshot objects every Instance
  of this Generation shares.
- Every one of these Instances released every descriptor and thread it opened.

It proves nothing about the network attachment seam, which no launch path can reach; see the
finding below. It proves nothing about latency, about the public Backend, about a jailed
`soma-vmm` worker, or about more than one host.

## Identities

- SOMA Git revision: `8ec0119`, whose code is what the run was made on; it was synchronised to eval-1 at `/srv/soma/agent-liveproof`.
- Host: eval-1, `Linux 6.8.0-138-generic` x86_64, 80 threads, XFS reflink scratch at `/srv`.
  Another agent's snapshot sweep ran on the same host throughout, so nothing here is a timing
  observation.
- Rust toolchain: `rustc 1.98.0 (88d9e12ae 2026-08-18)`, release profile.
- Kernel: `vmlinux-6.12.107-soma-v1`, SHA-256
  `f1af3a142fa39916cfac425a01b16b5f328279823533421c9eec3f192c05b746`.
- Guest agent: the `x86_64-unknown-linux-musl` release build of this tree, SHA-256
  `d6c54ad3b9f2192e7fe327489512285bb95b2e73cd87efcfb12c0a39b5a772de`. It is not the agent that
  was pinned before: this change adds the devpts mount without which no terminal can open.
- Image: `node:22` from the local OCI layout at `/srv/soma/oci-node22`.

## Invocation

```sh
SOMA_X86_64_VMLINUX=.../vmlinux-6.12.107-soma-v1 \
SOMA_EROFS_TOOLS=/srv/soma/fs-tools/erofs \
SOMA_GUEST_AGENT=.../soma-guest-agent \
SOMA_OCI_NODE_LAYOUT=/srv/soma/oci-node22 \
  cargo test --release --locked -p soma-kvm --test x86_64_snapshot_restore \
    -- --ignored --test-threads=1 --nocapture
```

## Result

```text
running 16 tests
test live::one_restore_reaches_ready_and_reports_the_node_version ... ok
test live::two_restores_of_one_snapshot_are_independent_instances ... ok
test x86_64_snapshot_restore_capability::context::a_command_sees_the_environment_directory_user_and_standard_input_it_was_given ... ok
test x86_64_snapshot_restore_capability::directory::a_directory_larger_than_one_listing_pages_and_every_entry_is_seen_once ... ok
test x86_64_snapshot_restore_capability::files::a_file_larger_than_one_record_round_trips_byte_for_byte ... ok
test x86_64_snapshot_restore_capability::files::a_host_write_is_read_in_the_guest_and_a_guest_write_is_read_by_the_host ... ok
test x86_64_snapshot_restore_capability::refusal::every_filesystem_refusal_names_the_reason_it_actually_happened ... ok
test x86_64_snapshot_restore_capability::secret::a_delivered_secret_is_whole_at_the_requested_mode_and_in_no_evidence ... ok
test x86_64_snapshot_restore_capability::terminal::a_real_terminal_echoes_reports_its_size_resizes_and_reports_its_end ... ok
test x86_64_snapshot_restore_certification::captured_candidate_certifies_promotes_and_reverifies ... ok
test x86_64_snapshot_restore_rejection::a_foreign_cpu_template_rejects_the_snapshot ... ok
test x86_64_snapshot_restore_rejection::a_tampered_object_is_rejected_before_any_vcpu_exists ... ok
test x86_64_snapshot_restore_rejection::sterile::a_head_of_the_wrong_shape_is_refused_and_the_worker_never_starts ... ok
test x86_64_snapshot_restore_rejection::sterile::an_unassignable_context_identifier_is_refused_and_the_worker_never_starts ... ok
test x86_64_snapshot_restore_rejection::the_published_objects_carry_no_launch_material ... ok
test x86_64_snapshot_restore_timing::warm_restore_timing_over_ten_iterations ... ok

test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 161.57s
```

The six new tests reported this, one restored Instance each:

```text
[files] the host read back 32 bytes: "written by a guest command at 0\n"
[files] one record carries at most 61353 bytes; the payload was 123943
[paging] the guest reported 1500 entries; the host took pages [1024, 476] for 1500 names
[refusal] absent=Failed(not found) status=Status { None } wrong_kind=Failed(wrong kind)
          not_empty=Failed(not empty) exists=Failed(already exists) emptied=Done
[context] the command reported:
a value only this run carries
/tmp/soma-live
65534
/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
--
one line the host typed
and a second

[pty] after open:
stty size
24 80
[pty] after resize:

# stty size
43 132
[secret] the guest reported: file="400 root 64" parent="755"
[secret] 64 value bytes appear in none of the console, the session digest, the placement
         rendering, or the three shared snapshot objects
```

The `[context]` block is one command's own standard output: the variable the caller named, the
directory it was told to run in, the account it ran as, the base `PATH` the caller did not name,
and then the two lines the host put on its standard input, copied back by `cat`.

The `[pty]` blocks are the terminal's own output: the shell's line discipline echoing what was
typed, and `stty` asking the guest kernel for the window size and getting first the size the
session was opened at and then the size it was resized to.

## The terminal had no devpts to open

The first run of this suite proved the terminal could not work at all. Every `PtyRequest::Open`
was refused, because `crates/soma-guest-agent/src/pty/device.rs` allocates a pair through
`/dev/ptmx` and then opens `/dev/pts/<n>`, while `crates/soma-guest-agent/src/boot.rs` mounted
devtmpfs, procfs, and sysfs and nothing else. With no devpts there is no slave to open, so the
whole terminal capability was unreachable inside a real guest while every component test passed
against the host's own devpts.

This change mounts devpts on `/dev/pts` in early init, before the device tree is moved into the
composed root, with `mode=0620,ptmxmode=0666` and without `MS_NODEV`, which a filesystem whose
whole purpose is device nodes cannot carry. The result above is the same suite after that fix.

## The network attachment seam is not reachable

`NetworkAttachment` in `crates/soma-kvm/src/x86_64/sandbox/restored.rs` is the one place a
per-Instance TAP descriptor and its leased MAC enter a restored machine. Nothing in this
workspace ever constructs one, so the seam could not be proved live and no live test for it was
written. Three facts, each checkable in the tree:

- The only `restore(RestoreRequest { .. })` call outside this crate's own tests is
  `crates/soma-local/src/backend/kvm/worker.rs:82`, and it passes `network: None` with a comment
  saying no bundle is assigned yet. Every Instance the launch path produces therefore keeps the
  loopback backend it was built with, and its link stays down.
- `soma_netd::receive_tap`, the function that would hand a VMM the descriptor of an assigned
  bundle, has no caller outside `crates/soma-netd`'s own tests and
  `crates/soma-netd/tests/live/checks.rs`. `soma-vmm` does not depend on `soma-netd` at all, and
  `soma-local` depends on both without joining them.
- Even joined, the descriptor would arrive in the wrong mode. `crates/soma-netd/src/tap.rs`
  opens `/dev/net/tun` through `OpenOptions` and never sets `O_NONBLOCK`, and no `fcntl` or
  `set_nonblocking` call exists anywhere in `crates/soma-netd`. Both
  `NetworkAttachment` and `TapBackend::new` in
  `crates/soma-kvm/src/virtio/devices/net/backend.rs` require an already non-blocking
  descriptor, because the device thread reads it inline; a blocking one would stall every other
  device on the bus.

Closing the gap needs a caller that receives the bundle's descriptor, makes it non-blocking, and
builds the `NetworkAttachment` the restore already accepts. Until then the seam is designed and
component-tested, and no run can say more about it than that.
