# The real soma-vmm worker running jailed - 2026-08-31

## Evidence boundary

This result proves that the `soma-vmm` binary, not the `jail-probe` stand-in, runs inside the jail the [VMM jail profile](../research/vmm-jail-profile.md) describes: the launcher execs it, it attests its own containment from inside, and it serves the lifecycle contract of `crates/soma-vmm` over the control descriptor it was handed.

Proved live by this run, with the production binary as the child:

- Only the manifested descriptors are open: slots 0, 1, 2, and the one control socket, and nothing else below `RLIMIT_NOFILE`; the executable slot is gone after `execveat`.
- The worker is PID 1 of its own PID namespace, runs as uid and gid 60001, and its six namespaces all differ from the launcher's; only `lo` exists in its network namespace.
- Its root is empty, is not writable, and has neither procfs nor sysfs.
- The launcher-side evidence records `no_new_privs`, seccomp filter mode 2, and empty effective and permitted capability sets.
- Its cgroup v2 leaf exists, reads back the written `memory.max` and `pids.max`, and contains the worker process.
- The worker narrows its own filter on request, after which the same attestation it had just served is killed with `SIGSYS` and recorded as a seccomp kill.
- It serves Launch, Execute, and Stop through the crate's `Machine`, replays an identical operation from its receipt rather than performing it twice, refuses an undefined packet without changing Machine state, leaves with the status it is told to, and leaves with its own supervisor-gone status when the control socket closes.
- Every worker test ends by reconciling the jail and proving the leaf, the jail root, and the process are gone.

It does not prove anything about machine restoration: this worker holds no platform, so its Launch fails at artifact verification by design and the honest failure receipt is what the test asserts. It also proves nothing about a transferred TAP endpoint, `/dev/kvm` in the worker's hands, `io.max`, snapshot ioctls, prepared workers, or any latency objective.

## Identities

- SOMA Git revision: branch `worktree-agent-ab6c43e5b89962685`, worktree of `e8bdbe2`; the run below was made on the tree that this change commits.
- Host kernel: `Linux 7.0.0-30-generic` x86_64, Ubuntu 24.04.4 LTS.
- Container runtime: Docker 29.3.0, `docker run --rm --privileged --cgroupns=private`, image `ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517`.
- Delegated subtree `/sys/fs/cgroup/soma-jail` reads back `cpu io memory pids`.
- Rust toolchain: `1.98.0 (88d9e12ae 2026-08-18)`, target `x86_64-unknown-linux-musl` for the worker, the probe, and the test binary.
- Worker: `target/x86_64-unknown-linux-musl/debug/soma-vmm`, 7,872,864 bytes, SHA-256 `a68eee4584cf970f69d3a217f602e70d0edbbcd3c713afc3658d74dea37e538b`, `file` reports `static-pie linked`; a debug build and a local measurement.
- Jail specification for the worker tests: uid and gid 60001, `memory.max` 64 MiB, `memory.swap.max` 0, `memory.oom.group` 1, `cpu.max` `100000 100000`, `pids.max` 8, `RLIMIT_NOFILE` 16, `RLIMIT_NPROC` 64, `RLIMIT_FSIZE` 1 GiB, `RLIMIT_CORE` 0, manifest `control`.

## Invocation

```sh
./scripts/jail-live-tests.sh
```

The script builds the worker, the probe, and the `jail_live` test binary for the musl target, refuses any of them that is not statically linked, and runs the ignored tests as root inside the container with `SOMA_VMM_BINARY` pointing at the worker.

## Result

```text
kernel: 7.0.0-30-generic
container identity: uid=0(root) gid=0(root) groups=0(root)
image os: ubuntu 24.04
delegated controllers: cpu io memory pids
crw-rw---- 1 root 993 10, 232 Aug 31 03:22 /dev/kvm

running 19 tests
test containment::cgroup_limits_read_back_and_contain_the_child ... ok
test containment::child_runs_as_the_ephemeral_identity_in_six_fresh_namespaces ... ok
test containment::forbidden_syscalls_are_recorded_seccomp_kills ... ok
test containment::kvm_version_is_admitted_while_tunsetiff_is_killed ... ok
test containment::root_is_empty_read_only_without_procfs_or_sysfs ... ok
test containment::sealed_table_hides_an_injected_descriptor ... ok
test containment::steady_state_keeps_threads_but_drops_setup_syscalls_and_ioctls ... ok
test failure::an_existing_leaf_fails_closed_and_is_never_reused ... ok
test failure::launcher_process_death_kills_the_child_and_recovery_releases ... ok
test failure::launching_thread_exit_kills_the_child ... ok
test failure::memory_max_oom_kills_the_whole_group ... ok
test failure::pidfd_identity_outlives_the_numeric_pid ... ok
test failure::pids_max_exhaustion_is_contained ... ok
test failure::stuck_child_is_killed_through_the_pidfd_at_reconcile ... ok
test failure::wrong_descriptor_kinds_fail_closed_before_seccomp ... ok
test worker::sealing_the_filter_turns_a_later_attestation_into_a_seccomp_kill ... ok
test worker::the_worker_attests_the_jail_it_was_launched_into ... ok
test worker::the_worker_leaves_when_its_supervisor_closes_the_control_socket ... ok
test worker::the_worker_serves_the_lifecycle_contract_from_inside_the_jail ... ok

test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 1.76s
```

## Host load and the two timing-sensitive stand-in tests

The suite was run six times on this host. The four `worker::` tests passed in every run. Two of the older `jail-probe` tests failed in three runs made while the host load average was near 30, and passed in every run made on a quiet host:

- `failure::memory_max_oom_kills_the_whole_group` and `containment::steady_state_keeps_threads_but_drops_setup_syscalls_and_ioctls` reported `child failure report unreadable: errno 110`, which is the ten-second child report deadline expiring before the loaded host scheduled the child through its pre-exec steps.
- `failure::launcher_process_death_kills_the_child_and_recovery_releases` observed the probe exiting with status 0 instead of dying of `SIGKILL`. The probe exits when its control socket reaches end of stream, and killing the launcher closes that socket at the same instant the parent-death signal is raised, so which of the two ends the probe is a kernel race. That behavior predates this change; the probe's end-of-stream exit is unchanged by it.

Neither observation involves the worker, and neither is claimed as a jail property here.
