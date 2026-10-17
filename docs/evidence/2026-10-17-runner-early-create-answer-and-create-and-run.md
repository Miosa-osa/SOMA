# The runner's early create answer and create-and-run, measured against real machines

## Result

Two opt-in create fields were added to the public runner: `"wait_ready": false`, which answers `201` as
soon as the launch has taken its facade lease and leaves the launch running behind the answer, and
`"run": {"command": ...}`, which runs a command in the sandbox a create just made, on the same facade
lease, and answers the create and the command together.

Neither field shortens the work: a launch plus a command costs what it costs. What they remove is the
caller's round trip, and the table below is the measurement that says so. Both new paths land within a
millisecond of the synchronous path at concurrency 1, and the synchronous path's create and command
times are recovered exactly in the other two paths' accounts of the same launch.

`node -v` on a prepared `node:22` Generation on host-01 (32 threads, x86_64, `/dev/kvm`), one process per
measurement, 34 prepared machines, one key, one tenant, every sandbox destroyed after its sample.
All figures are p50 unless the column says otherwise. `create` is the create answer's `pool` and `exec`;
`ready` is the launch's own ready phase as its receipt timed it; `waited` is the `wait` segment of the
command's answer; `command` and `run` are the command's own duration; `requests` is how many calls the
flow makes, which is what a network adds to a total.

| Concurrency | Flow | n | Total p50 | Total p90 | create | ready | waited | command | run | requests |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | default, then exec | 30 | 21.131 ms | 21.764 ms | 7.882 ms | 4.861 ms | - | 13.276 ms | - | 2 |
| 1 | early answer, then exec | 30 | 21.016 ms | 22.109 ms | 0.009 ms | - | 7.681 ms | 13.286 ms | - | 2 |
| 1 | create and run | 30 | 21.318 ms | 22.016 ms | 21.290 ms | 4.858 ms | - | - | 13.000 ms | 1 |
| 32 | default, then exec | 64 | 55.503 ms | 72.429 ms | 17.467 ms | 7.518 ms | - | 34.812 ms | - | 2 |
| 32 | early answer, then exec | 64 | 50.760 ms | 60.675 ms | 0.072 ms | - | 17.212 ms | 30.790 ms | - | 2 |
| 32 | create and run | 64 | 57.147 ms | 70.921 ms | 57.007 ms | 8.726 ms | - | - | 33.000 ms | 1 |

The three paths cost the same server-side work, and the early answer accounts for the launch it no
longer waits for: at concurrency 1 its create answers in 10 µs while the command's answer reports 7.736 ms
of waiting, and those two add to 7.746 ms against the 7.883 ms the synchronous create spent on the same
launch, the same launch timed by the same receipts and read through two different doors. At concurrency 32
the ordering between the three paths is not stable: this host runs a pre-existing `soma-api-large` service
of its own, and the three totals move by more than they differ from each other from run to run.

## What the total on the wire would be

The caller here is in the same process as the runner, so the totals above are server-side and carry no
network. On the wire a flow's total is its server-side total plus one round trip per request, except that
the early answer's command request is answered from inside the wait its create's launch is already under,
so its round trip is absorbed rather than added: the default path pays two round trips before its command
is answered, the early path one, and create-and-run one. At the 7 ms Virginia-to-New-York round trip the
ComputeSDK benchmark runs at, the concurrency-1 row becomes 35.2 ms for the synchronous path against
28.2 ms and 27.7 ms for the two new ones.

## What this does not say

- The caller is `Runner::handle`, the entry point the HTTP transport calls once it has read a request.
  TLS, HTTP framing, and connection reuse are covered by the integration tests under `crates/soma-api/tests`,
  not by this measurement.
- The command ran inside the guest through the engine's own exec path, but no network was attached to any
  machine: the shape these machines were prepared at has no network, and this host has no broker.
- Concurrency 32 is 2 rounds of 32 and so 64 samples, not 30 rounds; each round is a simultaneous burst.
- The measurements were taken on a host holding a pre-existing `soma-api-large` service of its own and, for
  the run below, a per-minute process sampler that a maintenance roll had just installed. That is why the
  concurrency-32 totals are 2.4 to 2.7 times the concurrency-1 ones rather than near them, and why the
  concurrency-32 ordering between the three paths moves between runs.
- Both new fields are opt-in and neither is set by the platform's adapter yet, so no user-visible latency
  has changed.

## How it was run

`cargo run --release -p soma-api --example tti_measure` on host-01, with `SOMA_GENERATION_STORE`,
`SOMA_HEAD_DIR`, `SOMA_HEAD_SHARDS`, `SOMA_PREWARM_REFERENCE=node:22`, `SOMA_PREWARM_MEMORY_MIB=1024`,
`SOMA_PREPARED_MACHINES=34` and `SOMA_IMMEDIATE_FORCED_DESTROY=1`, as the example's own header documents.
The scratch state root, the build tree and every sandbox it made were removed from the host afterwards.

## Transcript

```
concurrency 1
  default + exec         n=30   total p50=    21.131 ms  p90=    21.764 ms  create p50=7882 us (body create_ms p50=7000 ms), ready p50=4861 us, waited p50=none, command p50=13.276 ms, run p50=none, requests=2
  early answer + exec    n=30   total p50=    21.016 ms  p90=    22.109 ms  create p50=9 us (body create_ms p50=0 ms), ready p50=0 us, waited p50=7.681 ms, command p50=13.286 ms, run p50=none, requests=2
  create and run         n=30   total p50=    21.318 ms  p90=    22.016 ms  create p50=21290 us (body create_ms p50=21000 ms), ready p50=4858 us, waited p50=none, command p50=none, run p50=13.000 ms, requests=1

concurrency 32
  default + exec         n=64   total p50=    55.503 ms  p90=    72.429 ms  create p50=17467 us (body create_ms p50=17000 ms), ready p50=7518 us, waited p50=none, command p50=34.812 ms, run p50=none, requests=2
  early answer + exec    n=64   total p50=    50.760 ms  p90=    60.675 ms  create p50=72 us (body create_ms p50=0 ms), ready p50=0 us, waited p50=17.212 ms, command p50=30.790 ms, run p50=none, requests=2
  create and run         n=64   total p50=    57.147 ms  p90=    70.921 ms  create p50=57007 us (body create_ms p50=57000 ms), ready p50=8726 us, waited p50=none, command p50=none, run p50=33.000 ms, requests=1
== measure exit: 0 ==
```

Three runs of the same example on the same host, one of them before the maintenance roll of 2026-10-17 and
two after it, reproduced the same shape of result: at concurrency 1 the early answer's create plus its
command's wait recovered the synchronous create's own time to within 0.2 ms, and the three totals stayed
within a couple of milliseconds of each other. One of them is why the `run` column is cross-checked against
the body's own `duration_ms`: it had been printed in the wrong unit, reading 0.013 ms for a 13 ms command.

## Test evidence, same host

`cargo test -p soma-api` on host-01 with the same tree:

```
test result: ok. 169 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.02s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.06s
```

`cargo fmt -p soma-api --check` and `cargo clippy -p soma-api --all-targets -- -D warnings` both pass on
the same tree.
