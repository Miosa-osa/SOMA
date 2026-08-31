# The `soma machine` surface does not survive its launching process - 2026-08-31

**Superseded by [a `soma machine` sandbox that survives its launching process](2026-08-31-durable-machine-host.md),
measured the same day at `1cc7b87`.** Everything below is retained as the observation that was
made at `2082c03`; the closing paragraph names what closed it.

Measured on eval-1 at `2082c03`, KVM backend, busybox at one vCPU and 1024 MiB, against a
Generation compiled and captured by the same binary.

## What happens

`soma machine launch` succeeds and reports a ready sandbox:

```
{"command":"machine.launch","status":"ok",
 "result":{"instance_id":"9d2a81b5cca441d9a16147311e30ace2","state":"ready"}}
```

`soma machine exec --instance-id 9d2a81b5cca441d9a16147311e30ace2 -- /bin/echo hi`, run as a
**separate process**, refuses:

```
{"command":"machine.exec","status":"error",
 "error":{"code":"backend_unavailable","message":"backend capability is unavailable",
          "retryable":true}}
```

The machine and its guest session live in the process that launched them, so nothing outlives that
process for a second invocation to reach. `soma run`, which launches and executes inside one
process, is unaffected and is what every performance figure in this repository was measured with.

## Why this is worse than a failure

`launch` returns `status: ok` and `state: ready`, and hands back an instance identity. That
identity cannot be used by anything. A caller following the command line's own contract - launch,
keep the identity, exec against it later - gets a success it cannot act on, and the error it
eventually receives is marked `retryable: true`, which invites a retry loop that can never
succeed. The surface reads as durable and is not.

## What this explains

The contract benchmark under `benchmarks/local_alpha/burst` has reported 0 of 100 for every run.
Its boundary spans two `soma` process spawns by design, because that is what the providers it is
written against do. Per-slot records show the mechanism exactly:

| Process | Exit | Meaning |
| --- | ---: | --- |
| launch | 0 | `Success` - the machine really did launch, in 1.64 s under a hundred-way burst |
| exec | 76 | `CapabilityUnavailable` |
| destroy | 69 | `Conflict` |

Launch is not the failing step. **The benchmark is measuring precisely the capability that does not
exist yet**, and no shape, store, or harness change will move it off zero. A hypothesis that the
zeroes came from a shape mismatch - the harness defaults to 10240 MiB of storage while the
Generation was captured at 2048 - was tested by running it at the captured shape, and it still
reported 0 of 100. That hypothesis was wrong.

## What would close it

A machine hosted by a process that outlives the launching command, which is the jailed `soma-vmm`
already built and live-proved as a process but not yet hosting a machine. Until then the honest
statement is that SOMA has a working single-process sandbox and no working durable one, and the
`machine` subcommand should not report `ok` for a launch whose identity it cannot honour.
