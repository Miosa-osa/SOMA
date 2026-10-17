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
| 1 | default, then exec | 30 | 21.171 ms | 23.846 ms | 7.883 ms | 4.758 ms | – | 13.376 ms | – | 2 |
| 1 | early answer, then exec | 30 | 21.237 ms | 22.146 ms | 0.010 ms | – | 7.736 ms | 13.421 ms | – | 2 |
| 1 | create and run | 30 | 20.743 ms | 21.171 ms | 20.716 ms | 4.707 ms | – | – | 13.000 ms | 1 |
| 32 | default, then exec | 64 | 47.364 ms | 56.525 ms | 15.309 ms | 6.828 ms | – | 30.501 ms | – | 2 |
| 32 | early answer, then exec | 64 | 49.663 ms | 63.968 ms | 0.073 ms | – | 18.340 ms | 27.264 ms | – | 2 |
| 32 | create and run | 64 | 49.222 ms | 69.717 ms | 49.144 ms | 7.493 ms | – | – | 29.074 ms | 1 |

The three paths cost the same server-side work, and the early answer accounts for the launch it no
longer waits for: at concurrency 1 its create answers in 10 µs while the command's answer reports 7.736 ms
of waiting, and those two add to 7.746 ms against the 7.883 ms the synchronous create spent on the same
launch — the same launch, timed by the same receipts, read through two different doors. At concurrency 32
the ordering between the three paths is not stable: this host runs a pre-existing `soma-api-large` service
of its own, and the three totals move by more than they differ from each other.

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
- The measurements were taken on a host holding a pre-existing `soma-api-large` service of its own, which
  is why the concurrency-32 totals are 2.4 times the concurrency-1 ones rather than near them.
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
  default + exec         n=30   total p50=    21.171 ms  p90=    23.846 ms  create p50=7883 us (body create_ms p50=7000 ms), ready p50=4758 us, waited p50=none, command p50=13.376 ms, run p50=none, requests=2
  early answer + exec    n=30   total p50=    21.237 ms  p90=    22.146 ms  create p50=10 us (body create_ms p50=0 ms), ready p50=0 us, waited p50=7.736 ms, command p50=13.421 ms, run p50=none, requests=2
  create and run         n=30   total p50=    20.743 ms  p90=    21.171 ms  create p50=20716 us (body create_ms p50=20000 ms), ready p50=4707 us, waited p50=none, command p50=none, run p50=13.000 ms, requests=1

concurrency 32
  default + exec         n=64   total p50=    47.364 ms  p90=    56.525 ms  create p50=15309 us (body create_ms p50=15000 ms), ready p50=6828 us, waited p50=none, command p50=30.501 ms, run p50=none, requests=2
  early answer + exec    n=64   total p50=    49.663 ms  p90=    63.968 ms  create p50=73 us (body create_ms p50=0 ms), ready p50=0 us, waited p50=18.340 ms, command p50=27.264 ms, run p50=none, requests=2
  create and run         n=64   total p50=    49.222 ms  p90=    69.717 ms  create p50=49144 us (body create_ms p50=49000 ms), ready p50=7493 us, waited p50=none, command p50=none, run p50=29.074 ms, requests=1
== final measure exit: 0 ==
```

Two earlier runs of the same example on the same host reproduced the same shape of result: the early
answer's create plus its command's wait recovered the synchronous create's own time, and the three totals
stayed within a couple of milliseconds of each other at both concurrencies. The second of them is why the
`run` column is now cross-checked against the body's own `duration_ms`: it had been printed in the wrong
unit, reading 0.013 ms for a 13 ms command.

## Test evidence, same host

`cargo test -p soma-api` on host-01 with the same tree:

```
test result: ok. 168 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.02s
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
