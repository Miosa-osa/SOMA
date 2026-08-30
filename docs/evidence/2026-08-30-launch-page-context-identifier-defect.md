# The launch page context identifier defect - 2026-08-30

## Capability status: Live-proved at `c0fd993`

Every restored sandbox failed to form a session between `f614458` and `c0fd993`.
This record states the defect, its root cause, the repair, and what each of those is proved by.

## What failed

`f614458` gave each Instance a context identifier derived from its `InstanceId`, so that two concurrent sandboxes could not be given the same one.
It changed the identifier the machine was built with and did not change the identifier the launch page carries, which remained the constant the derived value replaced.

The guest agent compares the identifier its own vsock device reports against the identifier the launch page names, and refuses the session when they disagree.
A restored machine therefore reached its repair point, accepted its launch material, repaired its entropy, and then died:

```text
soma-guest-agent: poisoned by Transport after [Captured, MaterialAccepted, EntropyRepaired]
```

The failure is deterministic rather than intermittent, and it consumed the whole boot deadline before the caller saw `backend_failure`.

## Root cause

`link_down_network` in `crates/soma-local/src/backend/kvm/boot.rs` built the launch page's network identity from a literal `3`.
`guest_cid_for` built the machine from the Instance.
Nothing related the two values, and no test compared them.

The check the guest performs is the correct one and is not what changed: binding the transport a session runs over to the Instance the launch page names is what makes the identifier part of Instance authority rather than a convenience.
The defect is that the host named two different identifiers for one machine.

## Repair

The launch page is now built from the identifier the machine was given.
The regression test `the_launch_page_names_the_machine_s_own_context_identifier` compares the two values, which is the whole of the defect.

Run against the previous code the test fails with the two identifiers it is meant to catch:

```text
assertion `left == right` failed
  left: 3
 right: 2312835370
```

Run against `c0fd993` it passes, with the workspace at 143 test suites and no failures.

## Live proof

At `c0fd993`, on the development host described in [the restore stage timeline](2026-08-30-x86_64-restore-stage-timeline.md), `soma --backend kvm run node:22 -- /usr/local/bin/node --version` restored a prepared snapshot, formed the authenticated session, and returned `v22.23.2` from inside the guest.

One sample. It proves that a restored Instance reaches a working session again; it is not a latency result and no timing in it may be quoted as one.

## What this record does not prove

- It contains no accepted latency, throughput, or capacity result.
- It does not prove cleanup: the same run could not prove cleanup, for the separate reason recorded in [the restore stage timeline](2026-08-30-x86_64-restore-stage-timeline.md).
- The console output of a failing run was observed but not retained, so the quoted agent line above is reproduced from the repair commit rather than from a retained artifact. The regression test, which is retained and runnable, is what supports the root cause.
- It says nothing about whether any earlier measurement is affected. Measurements taken before `f614458` used one constant identifier for both the machine and the launch page, so the two agreed and the defect did not exist on that code.
