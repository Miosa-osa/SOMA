# macOS was classified as hosting a machine it does not hold - 2026-09-01

## Capability status: Component-tested. Not live-proved, and it cannot be from this session

There is no macOS host reachable from the session that made this change. eval-1 is Linux
x86_64 and the `soma-local` macOS backend does not compile off macOS at all. Nothing below is a
run. Every statement is either a reading of code that is in this repository or a test that
executes on this host, and the one claim that would need a Mac to settle is named as unsettled
at the end.

## What was believed

`soma_local::machine_hosting(BackendKind::MacosVirtualization)` answered `LaunchingProcess`,
and that answer is load-bearing: `soma machine launch` refuses on it with `machine_not_hosted`
and exit 76, and `POST /v1/sandboxes` answers 501 `capability_unavailable`. The reason given,
in [the honest status surface](2026-08-31-honest-status-surface.md), was:

> The KVM and macOS backends hold the machine and its authenticated guest session in the process
> that launched them.

That sentence is true of KVM as it was written and false of macOS. The document proves its case
with a docker cohort and contains no macOS observation of any kind; macOS was swept in with KVM
by resemblance. The engineering standard's mechanism-claim rule exists for exactly this.

## What the code does

The macOS adapter holds no machine. `MacOsBackend` is an executable path and a process runner
(`crates/soma-macos/src/backend/mod.rs`), and `MacBackend` adds a clock and a set of instance
names whose create already failed (`crates/soma-local/src/backend/macos/adapter.rs`). Neither
carries a file descriptor, a memory mapping, a thread, or a session belonging to a machine,
because there is nothing of that kind to carry: every operation spawns one `container` process
and reads its output.

Four facts settle it, and each is one file:

1. **The machine is registered with a service this process does not own.** `probe` runs
   `container system status --format json` and refuses unless it answers `running`
   (`crates/soma-macos/src/backend/probe.rs`). SOMA neither starts nor stops that service, and
   the version probe names `container-apiserver` as a component beside the CLI. The machine
   lives there.
2. **It is registered under a name derived from the Instance.** `create` passes
   `--name soma-<instance_id>` and `--label io.miosa.soma.instance=<instance_id>`, both
   computed from the identity alone (`crates/soma-macos/src/request/identity.rs`).
3. **Every later operation re-finds it by that name.** `inspect_owned` runs
   `container inspect soma-<instance_id>` and verifies the record's own id and ownership label
   against the identity before the operation proceeds
   (`crates/soma-macos/src/backend/ownership.rs`). Ownership is re-proved from the service's
   record on each call rather than remembered.
4. **One launch already survives several process deaths.** `MacBackend::launch` runs create,
   start, and inspect as three separate `container` processes
   (`crates/soma-local/src/backend/macos/lifecycle.rs`). The machine outlives the first two
   before the launch has even returned.

This is the same shape the ledger already credits Docker with, down to the container name:
`soma-{instance}` in `crates/soma-local/src/backend/docker/container.rs`, `soma-{instance}` in
`crates/soma-macos/src/request/identity.rs`. Both are addressed by identity through a runtime
service; the classification disagreed with itself.

## Why no machine host was built

The gap this closes was recorded as "the same host-process work done for KVM". It is not. A KVM
sandbox is descriptors, a guest memory mapping, a vCPU thread, and a Noise session, all of which
belong to one process, so surviving a command required a process to hold them. A macOS sandbox
is a record in `container-apiserver`. A host process for it would hold nothing, duplicate a
service Apple already ships and the probe already requires, and add a hop to every call. Building
one would have been cost with no capability behind it, which is the same judgement the declared
device set applied to a network device whose link can never come up.

## What changed

`crates/soma-local/src/backend/mod.rs`: the macOS arm joins the other three.
`MachineHosting::LaunchingProcess` is now answered by no backend, and its documentation says so,
because a refusal nothing triggers should read as a guard for the next backend rather than as a
live branch. The refusal path itself is untouched: `not_hosted`, `machine_not_hosted`, exit 76,
and the `DurableMachineHosting` capability error all still exist and are still tested
(`a_launch_this_process_cannot_host_is_refused_rather_than_reported_ready`).

Two tests changed or arrived.

`the_backends_that_host_a_machine_only_in_this_process_are_named` became
`every_backend_hands_back_an_identity_a_later_process_can_use`, and now covers `Remote` as well,
so a fifth backend that hosts in-process fails the build rather than inheriting an answer.

`a_machine_is_driven_from_processes_that_did_not_create_it`
(`crates/soma-macos/src/tests/lifecycle/hosting.rs`) is the mechanism rather than the outcome.
Three backend values are built from scratch, each with its own scripted process runner, and none
ever sees the one before it. The first creates and starts; the second, holding nothing but the
Instance identity, runs a command; the third stops and deletes. It then asserts that all three
addressed `soma-<instance>`, and that the second and third re-proved ownership with
`inspect soma-<instance>` before acting. It runs on Linux, because `soma-macos` is portable Rust
that shells out rather than an Apple-framework binding.

## Public contract change

`soma machine launch --backend macos` exits 0 where it exited 76, and `POST /v1/sandboxes`
answers 201 where it answered 501, on a macOS host with the Apple runtime running. This is a
visible change to anything scripted against the refusal.

## What is still unproved

That a `soma machine launch` on macOS, followed by `soma machine exec` from a second process
against the returned identity, succeeds end to end. The mechanism above says it must, and every
step of it is the step Docker takes, but the run has not happened and no host in reach can make
it happen. It is not claimed. The proof it needs is the one
[the durable machine host](2026-08-31-durable-machine-host.md) records for KVM: five separate
`soma` processes driving one sandbox, with a file written by the second read back by the third.

Two smaller things a Mac would also settle. `MacBackend::already_cleaned` is per-process, so a
machine whose create failed and was force-deleted inside process A is not known to be gone in
process B; B's cleanup would inspect, find nothing, and map that to a failure rather than to a
complete release. And the adapter is pinned to Apple `container` `>=1.3.0,<1.4.0`, which no run
here has exercised.
