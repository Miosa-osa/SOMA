# ADR 0042: API process owns machine-host exit collection

## Status

Accepted.

## Context

Hosted KVM launch creates one machine-host child and transfers sandbox lifecycle to that process after an authenticated startup handshake.
Dropping Rust's `Child` handle does not collect the eventual Unix exit status.
A long-running API therefore accumulated zombie children after successful sandbox destruction even though guest resources were cleaned correctly.

## Decision

The API process owns exit collection for every machine-host child it launches successfully.
After the startup handshake succeeds, launch transfers the `Child` handle to one process-wide reaper thread.
The adoption queue is bounded at 1,024 handles.
When the queue is full, adoption waits only until the live reaper drains one queue slot and takes ownership, never until the machine exits.
When the queue is disconnected, adoption terminates and collects the child instead of returning an Instance whose exit nobody owns.
The reaper polls running children every 10 ms, removes collected exits, and waits synchronously when `try_wait` returns an indeterminate error.

The reaper lives for the API process lifetime.
It does not own guest cleanup, durable Instance state, or machine-host termination.
Those responsibilities remain in the lifecycle and reconciliation paths.
Normal API process exit lets the operating system reparent any still-running machine hosts, while durable reconciliation remains responsible for recovering their Instance state after restart.

## Consequences

Successful create and destroy cycles no longer leave zombie machine-host processes under a persistent API parent.
Ordinary launch cannot stall on exit collection while the bounded queue accepts ownership.
Exceptional queue saturation can delay adoption by one reaper drain interval, while reaper loss terminates the affected machine before Launch can report it ready.
Qualification must inspect the API parent after a completed burst and prove that no exited children remain.

## Addendum 2026-10-12: a claimed machine host leaves the API unit's control group

### Context

This ADR intended that an API restart leaves hosted machines running: the operating system reparents them and durable reconciliation recovers their state.
The production unit defeats that.
`soma-api.service` uses `KillMode=mixed` with `TimeoutStopSec=30s`, so a stop or restart sends `SIGTERM` to the main process and then `SIGKILL` to everything left in the unit's control group.
Every machine host is a child of the API and starts in that group, so a restart kills every hosted sandbox.

Three questions decided the design.

1. Who holds the machine and its guest session?
   The machine host.
   An unjailed machine host builds a `KvmBackend` with `Held::Resident`, so the KVM descriptors, the guest memory mapping, the vCPU thread, and the authenticated session all belong to that one process.
   The API holds none of them.
2. How does a restarted API reach a machine that is already claimed?
   By identity, with no in-memory table.
   A hosted backend addresses the machine through its Instance socket under the hosted directory (`host::channel::connect`, `liveness`, `inspect`, execute, cleanup).
   A new API process finds a surviving machine the same way the old one did.
   After it reports `Ready::Launched` the host never uses its standard output or the handoff socket again, so nothing ties it to its parent.
3. What does the runner do with a recovered sandbox?
   `recover_sandboxes` lists the durable store and re-adopts each Active or Executing entry whose machine name carries the tenant label the runner wrote at create.
   An entry without the label was not created through the runner and stays unreachable through it.
   Its creation time is not recorded, so a recovered sandbox gets the runner's default idle timeout.
   The listing must succeed for any of this to run, which is why a failed launch's unreadable receipt mattered (SOMA PR #65).

### Decision

A machine host that has received its launch capability moves itself, before it launches the machine, into a leaf of the delegated cgroup subtree named by `SOMA_JAIL_CGROUP_ROOT` (`soma-<instance>-live`).
That is the leaf a live CPU resize already creates, so a later resize finds it and writes `cpu.max` there.
The subtree is a sibling of the API unit's control group, so the unit's kill does not reach it.
The production hosts already set the variable and delegate the subtree (`deploy/soma-api-host`, miosa-compute #2035).

A sterile host that has claimed nothing stays in the unit's group and ends with the unit.
The API refills its pool on start.

The move is best effort.
If no subtree is configured, or the move fails, the host still serves its machine and the machine simply does not outlive a unit stop.
A machine host has no log of its own, so the outcome is observable only from the machine's cgroup path.

### Accounting and limits

Leaving the unit's control group also leaves the unit's accounting and any `MemoryMax=` or `TasksMax=` it sets, so a machine would otherwise escape both.
The delegated subtree therefore enables the `memory` and `pids` controllers beside `cpu`, and carries the unit's limits as limits on the subtree root: `soma-live-cgroup.sh`, which runs before the unit starts, writes the unit's finite `MemoryMax=` and `TasksMax=` to the root's `memory.max` and `pids.max`.
The limit is on the root and not copied onto each leaf, because the unit's limit is shared by everything in it and a copy per machine would turn one shared cap into one cap each.
The surviving machines are then bounded together, separately from whatever the unit's remaining processes use.
The production unit sets no `MemoryMax=` and `TasksMax=infinity`, so today the root is unlimited and the gain is that the machines are counted under one cgroup.
A host reports whether its leaf is accounted (the controllers are enabled in the subtree), below.

### Races and visibility

Sweeping removes empty `-live` leaves, and a new leaf is empty until the host moves into it.
Sweep, create, and move therefore run under one exclusive lock on the subtree's root directory (a cgroup directory cannot hold a lock file), and the create and move retry as well.
A live CPU resize takes the same lock.
A machine host has no log, so its launch answer carries `live_cgroup { joined, accounted }` when a subtree is configured and the API logs a line when either is false.
An unconfigured host says nothing.

### Upgrade order

The launch answer is a line the host writes and the API reads, and the answer type refuses unknown fields.
The API starts a host from its own executable (`std::env::current_exe`, which on Linux is the image the API itself runs), so a newer host never sits under an older API: an older API keeps spawning its own binary until it restarts.
A host that survived a restart is addressed over the Instance socket, which this change does not alter.
The new field is written only when a live subtree is configured and is read with a default, so a newer API still reads an answer without it.

### Who can set the owner label

The runner re-adopts a sandbox because its machine name is `t-<tenant>`.
The name is set in exactly one place, the runner's create path, from the authenticated tenant. The runner refuses a create body that carries `name` (`RUNNER_UNSUPPORTED_REQUEST`, 400, before the facade is reached), and a runner test (`the_owner_label_comes_from_the_authenticated_tenant_never_from_the_request`) pins that the launch carries the authenticated tenant's label.
The direct soma-api listens on loopback only, and the control plane's SOMA host commands (`wire_request.ex`) carry no machine name at all.
So the label cannot be chosen by a tenant.
A machine launched directly with a `t-` name (as the proof does, on loopback) is recovered as that tenant's; that path is operator-only.

### Consequences

- Restarting or upgrading `soma-api` no longer ends claimed sandboxes. The new process finds them by Instance socket, the runner re-adopts the labelled ones, and every other path addresses them by identity.
- A machine host now outlives its parent for the 30-minute idle ceiling at most when nobody addresses it (a liveness probe connects and drops without counting as activity) (`IDLE_CEILING` in the serve loop), so an abandoned machine is still bounded.
- A stop of the unit is no longer a way to clear all sandboxes. Clearing them is the destroy path or an operator removing the live subtree.
- Empty `soma-*-live` leaves are swept by the next machine host that creates one and when the unit starts; after a destroy storm with no later launch they persist, harmlessly, until then.
- Sandboxes not created through the runner (no label) survive a restart too, for up to the idle ceiling, untracked by the runner meanwhile.
- Without `SOMA_JAIL_CGROUP_ROOT` nothing changes, which keeps development and CI as they were.

### Proof

A scratch API unit on a SOMA host, beside production and with its own port, state root, and cgroup subtree, creates a claimed machine, restarts, and must show: the machine-host pid is unchanged, its cgroup path is the live leaf and not the unit's, execute still works, the list names the instance with `host: live`, and the tenant-labelled instance is recovered by the runner.
The same sequence against the unmodified binary must show the machine gone.
