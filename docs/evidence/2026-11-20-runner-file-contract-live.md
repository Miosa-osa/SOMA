# The platform file contract through the public runner, against a real guest

## Result

A sandbox created through `Runner::handle` on the local KVM runtime answered every platform file call a client makes, as the contract in [ADR 0056](../adr/0056-runner-serves-the-platform-file-contract.md) requires.
A 9,437,261-byte file of pseudo-random bytes was uploaded as a `multipart/form-data` body in 64 KiB pieces, downloaded as a stream, and read back through the JSON read, and all three were identical to what was sent.
`sha256sum` run inside the guest reported the same digest the runner computed before sending, so the guest holds the bytes the client sent, not only bytes the runner can return.
The sandbox was destroyed and left no file in the head directory and no process naming the run's state root.

## How it was run

The run was made on `miosa-host-12` (96 threads, x86_64, Linux 7.0.0-43-generic, `/dev/kvm`), from a scratch directory of its own.
It was first run by hand on the pull request's branch, and then on 2026-11-21 through `scripts/end-to-end-check.sh` at `84583b7`, the merged commit on `r3/soma-unified`; that run is the one this record stands on (see "Through the end-to-end check" below).
A `node:22` Generation was prepared at 1024 MiB of memory and 4096 MiB of storage and certified by capturing its snapshot:

```sh
scripts/build-soma.sh
SOMA_ALLOW_COLD_BOOT_ENTRY=1 scripts/prepare-generation.sh node:22 <work>/prepared <fs-tools> 1024 4096
cargo build --release -p soma-local --example capture_snapshot
target/release/examples/capture_snapshot <work>/prepared/ref-<digest> 1024
cargo build --release -p soma-api --example runner_files
SOMA_GENERATION_STORE=<work>/prepared SOMA_HEAD_DIR=<work>/heads \
    target/release/examples/runner_files <work>/state node:22 1024 4096
```

The example (`crates/soma-api/examples/runner_files`) drives `Runner::handle` with the requests the transport would hand it, including the upload's body arriving through the same `StreamedBody` channel the transport feeds.
TLS, HTTP framing and the three HTTP versions are covered by `crates/soma-api/tests/runner/files.rs`, not by this run.

## Transcript

```
captured .../prepared/ref-8a02eda372743655dfbe5f03e83a19ea8b6c35a44be41aea679db02fa4c72a35/snapshot
  generation sha256:74a6dbb23eda87ae007878c3a00741c203412e7bfccf02e986f2e86c15fcc956
  memory 1073741824 bytes
  overlay 4294967296 bytes
  state 9086 bytes
create           201    88.537 ms
mkdir            201     1.963 ms
write            201     2.477 ms
list             200     0.718 ms
read             200     0.605 ms
upload           201   766.496 ms
download         200   566.884 ms, 9437261 bytes, complete true
read binary      200   600.202 ms
exec sha256sum   200   208.401 ms
rename           200     2.099 ms
stat old         404     0.492 ms
stat new         200     0.470 ms
chmod            200     0.480 ms
stat mode        200     0.457 ms
delete           200     1.326 ms
stat deleted     404     0.451 ms
destroy          200    43.254 ms
runner files: every call answered as the contract requires; 9437261 bytes uploaded, downloaded and read back identical, sha256 bbd5c18d25c13ea0443030332c69a4becef352e3f11bd02156d5a87c6abf9531 in the guest
heads_left=16
(the branch commit of the first hand run)
Linux 7.0.0-43-generic x86_64
96
```

`heads_left=16` counts the head directory's sixteen shard directories; a search for regular files under it found none after the run, and no process named the run's directories.

## What this does not say

- It is one sample of each call on one host and one Generation, so no time above is a measurement of anything; the times are printed so a reader can see which calls moved data.
- The machine had no network: the runner launched it with no public egress, because this host has no network broker.
- Nothing here reached the runner over the network or through the platform's routing; that routing is the platform's work.

## Through the end-to-end check

`scripts/end-to-end-check.sh --fs-tools <fs-tools>` at `84583b7` on `miosa-host-12`, as an unprivileged user, ended with every stage passed (rc=0 after 497 s):

```text
passed host_setup          kvm=ok virt=ok cgroup2=ok skopeo=ok cargo=ok python3=ok docker=ok musl=ok fs_tools=ok head_fs=xfs
passed prepare_generation  candidate=1547B store=6.5G
passed capture_snapshot    state=12K overlay=4.0G
passed run_sandbox         stdout='v22.23.5' exit=0 isolation=hardware_virtual_machine ready=25.2ms finished=89.3ms
passed runner_files        every call answered as the contract requires; 9437261 bytes uploaded, downloaded and read back identical, sha256 bbd5c18d25c13ea0443030332c69a4becef352e3f11bd02156d5a87c6abf9531 in the guest
passed large_command       heredoc of 12329 bytes: stdout='heredoc lines 231' exit=0
passed cleanup_proof       overlay_heads=clean processes=clean netns=clean nftables=unverified state_root=clean
```

`nftables=unverified`: the unprivileged user could not list the tables, so nothing was compared there.
