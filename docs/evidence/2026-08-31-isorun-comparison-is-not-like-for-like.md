# The Isorun comparison compares two different products - 2026-08-31

Every SOMA speed figure has been placed beside "Isorun: 22 ms sequential, 73 ms at concurrency
100". This records why that pairing is invalid, at a level deeper than the timing-boundary problem
the existing evidence already warns about.

## The known objection, already on record

[The Isorun creation telemetry record](2026-08-30-isorun-create-latency.md) is explicit:
`create_ms` is a vendor-reported field whose timer endpoints are undocumented, the harness wall
clock includes transport from another continent, and the document states in its own words that it
"proves nothing about SOMA" and that the figure must not be presented "beside a SOMA `Ready` figure
as though the two measure the same interval."

That instruction has been violated repeatedly in working conversation, and
[the host-class projection](../research/host-class-and-burst-projection.md) carries a table row
reading `| Isorun, for comparison | unknown | 22 ms | 73 ms |` in the same columns as SOMA's
measured numbers, guarded only by a line saying the table is not for reporting.

## The deeper objection, not previously recorded

Read what the harness actually does (`benchmarks/competitive/isorun_create_latency.py`). One
sample is three separate HTTP calls:

```
POST   /v1/runs             -> returns an id
POST   /v1/runs/{id}/exec   -> a command against that id
DELETE /v1/runs/{id}
```

The create call returns an identity, and the command is a **separate request against that
identity**. That is the persistent-sandbox shape: create once, address it repeatedly, destroy when
finished. It is what a coding agent needs, and it is what the ComputeSDK provider contract
describes.

**SOMA cannot do that at all.** `soma machine launch` returns an instance identity that is already
dead, because the machine lives inside the launching process
([evidence](2026-08-31-durable-machine-across-processes.md)). Every SOMA performance figure in this
repository, 31.31 ms sequential, 35.5 ms at concurrency 100, the whole speed ladder, the entire
`ready` segment analysis, was measured with `soma run`, which launches, executes, and destroys
inside one process and never produces a reusable identity.

So the two sides of the comparison are not the same operation measured differently. They are
different operations:

| | Isorun sample | SOMA measured path |
| --- | --- | --- |
| Create | returns a durable id | no durable id exists |
| Command | separate call against the id | same process, same invocation |
| Reuse | further commands can address the id | impossible |
| What is being timed | standing up an addressable sandbox | a one-shot process |

A one-shot process can legitimately be faster than standing up an addressable sandbox, because it
never has to make the sandbox addressable. The comparison flatters SOMA by exactly the amount of
work it has not implemented.

## What follows

1. No SOMA figure may be printed beside an Isorun figure until SOMA's measured path is
   create-then-command-by-identity, matching the flow and not only the timing boundary.
2. When the durable path works, the figure that may be compared is the one measured through it,
   which will be slower than the one-shot figures recorded here, and that is the honest number.
3. The `| Isorun, for comparison |` row is a reporting hazard even inside a document that
   disclaims reporting, and should carry the flow mismatch beside it rather than only the unknown
   host class.

Nothing in this record impugns the retained Isorun samples, which were collected carefully and
whose limits their own document states accurately. The failure is in how they were subsequently
quoted.
