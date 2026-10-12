# ADR 0052: Prewarm targets are set at runtime, per shape

## Status

Proposed.

## Context

A host keeps identity-free machines prepared so a launch skips restore and device construction.
Today that is one shape per host, named by `SOMA_PREWARM_REFERENCE` and `SOMA_PREWARM_MEMORY_MIB`, with a count taken from the worker setting once at start.
Nothing can change it without restarting soma-api, and a restart drops every prepared machine and every live sandbox's runner.
The pool planner needs to set how many machines are prepared for each shape, read back what exists, and do it for several shapes on one host.

## Decision

1. A prewarm target is `{reference, memory_mib, count}`: the OCI reference of a prepared Generation, the boot memory of the machines kept for it, and how many.
   This is the key the sterile pool already uses (`PrimedKey`: reference, Generation, boot memory), so a prepared machine serves exactly the launches it was built for.
2. The pool keeps a list of targets instead of one plan.
   Refill works through the list, builds what is missing, and retires what is surplus or no longer wanted.
   A launch claims a machine primed for its key, then an unprimed spare, then builds its own and says so, as before.
3. `PUT /v1/prewarm` on the loopback listener replaces the whole desired set and is idempotent; it answers the readback.
   `GET /v1/prewarm` answers the desired set and what exists.
   The routes are loopback only, like the rest of the control surface; the control plane reaches them through its host command road.
4. A request is refused by name and applied whole or not at all:
   - `unknown_reference` when no prepared Generation matches,
   - `invalid_target` for a zero or out-of-range memory or count, a duplicate key, or more than 16 targets,
   - `prewarm_budget_exceeded` when the sum of `count * memory_mib` is above the host budget, with the budget and the request in the answer.
   The budget is `SOMA_PREWARM_BUDGET_MIB`, default 60 percent of the host's memory.
5. The desired set is persisted under the state root (`prewarm-targets.json`, written to a temporary file and renamed) and read at start.
   A persisted set wins over the environment defaults; with none, the environment's single shape applies exactly as before.
   A persisted set that cannot be read is reported in the readback (`source: "env_default"`, `persisted_error`) and the environment defaults apply, never a silent partial set.
6. The readback per target is `target`, `prepared` (ready now), `building`, `last_failure`, and the counters `claimed_primed` and `claimed_miss` (launches that found no prepared machine for the key).
   The planner sizes from the miss counter.
   The answer also carries the unprimed spares, the budget, the committed memory, and the source of the set.

## Consequences

- Shrinking a target retires the surplus machines, so a smaller number frees memory at once instead of at the next restart.
- Machines are built one at a time in the background; a large increase converges over time and the readback shows the progress, which the planner must poll instead of assuming.
- An older soma-api has no route; a caller learns that from the 404 and from the control plane's capability check before it sends anything.
- Prewarm changes no sandbox that is already running.
- Shrinking kills only unclaimed machines.
  A machine leaves the pool under the lock that the shrink scans under, so it belongs to exactly one of them: a claim that wins owns a whole live machine, and one that loses finds none and builds its own, which the readback counts as a miss.
  A claimed or mid-claim machine is never terminated.
  `a_claim_racing_a_shrink_wins_a_live_machine_or_gets_a_miss` races claimers against a shrink and checks every machine is accounted for once.
- A set replaces everything, including the unprimed spares: with a persisted set the pool keeps primed machines for its targets and no spare processes.
  A host that ran with spares and no `SOMA_PREWARM_REFERENCE` loses them at the first set, by design.
- A persisted set is built in the background after start, so launches in the first moments can miss until the refill catches up; the readback shows it.
  Only the environment's single shape is prepared before traffic.
- A shape that cannot be built records the reason of the step that failed and backs off (5 s, doubling to 60 s) while the other shapes are built.
  A machine whose build finishes after the set changed is kept only if the set still wants it, checked under the pool's locks, so a shrink is never undone by a late build.
- `uncovered_demand` carries `boot_memory_mib` and `targetable`: a target can only name the Generation's boot memory, and demand is keyed by that memory in the hosted-machine path.
- A store that cannot be read, or an entry that is linked, ambiguous or damaged, is `prewarm_store_unavailable` (503) with the reason, not `unknown_reference`.
- `handle` (the sandbox handler) answers 404 for the prewarm routes; only the connection path serves them.
- The persisted set lives in the state root beside the records, and the state store's enumeration skips it (and its temporary file) by name.
  Enumeration treats every other stray entry as corruption and fails the whole `LIST`, so a file the store did not know about would have made `LIST` fail with `state_store_failure` on any host with a persisted set.
  A link or directory under that name is still corruption.
