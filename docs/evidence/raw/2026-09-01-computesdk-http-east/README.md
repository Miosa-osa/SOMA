# Raw exact HTTP Burst TTI artifacts

This directory retains three legacy synchronized 100-sandbox qualification cohorts, the initial secure cohort, and the later sterile-host qualification series.
The sterile series includes fixed 40/24/36 cohorts and later placement probes whose exact split is retained in each shard's sample count.
Every sample starts immediately before `POST /v1/sandboxes`, executes `/usr/local/bin/node -v` through `POST /v1/sandboxes/{instance}/commands`, stops timing after the successful command response, and destroys the sandbox outside the timing boundary.

The files named `run-N-hostXX.json` are the original host shards.
The files named `run-N-combined.json` are deterministic recomputations over the three raw shard sample lists.
The combined files do not calculate one cross-host wall clock because host-local monotonic clocks do not share an origin.
The v1 shard writer did not retain the common future release epoch used by the live runner.
These artifacts therefore do not independently prove cross-host synchronization, and the current combiner deliberately refuses them as new evidence.

The files named `run-secure4-hostXX.json` and `run-secure4-combined.json` are the final secure cohort.
They retain the common release epoch, contain exactly 100 canonical unique Instance identities, and prove 100 command successes plus 100 complete cleanups.
Their 1/1/98 placement followed observed host capacity after host03 and host04 returned intermittent `backend_unavailable` failures under higher weights.
The final combined result is median 109.65 ms, p95 149.33 ms, and p99 157.23 ms.
Its executable identity is SHA-256 `a8c6d03be5cdad14e7c29022da6e53f405ed4c3d99953efac56544901e502f4f`.

The three `run-N` cohorts used a removed size-only installed-artifact shortcut.
They are retained to explain the performance investigation but are not release or security evidence.

The `sterile/` directory holds ten secure cohorts.
Every cohort retained a common release epoch, 100 unique canonical Instance identities, 100 successful `node -v` commands, and 100 complete cleanups.
Runs 1 through 5 used binary SHA-256 `6322445ccfa6e7d5ff5e12c2544700fed210c63d2c4e8080c88df4e30ab53383`.
The first cohort followed executable replacement and measured 78.51 ms median.
The next four medians were 62.32, 69.34, 67.42, and 68.32 ms.
The 62.32 ms result is the best demonstrated capability, while the following variance prevents treating it as a stable public claim.
Runs 6 through 10 used the fully validated binary SHA-256 `ebeaadfaee2902547399969b7e0d27cd38a8c3849f59524157bae18dc4b98850`.
Their medians were 70.96, 69.41, 70.25, 66.43, and 69.23 ms.
Run 7 normalized all three hosts to the AMD P-state `performance` governor and reduced the repeated 40/24/36 tail to 77.50 ms p95 and 79.56 ms p99.
Runs 8 through 10 mapped host-specific contention knees, with 40/20/40 producing the best validated tail of 76.14 ms p95 and 79.21 ms p99.

The statistics apply the current ComputeSDK scoring implementation's five-percent trim from both ends, arithmetic median, and nearest-rank p95 and p99.
The inspected upstream revision was `46dea652fcc372e5acea0c9f372613d86b4b6bab` from `computesdk/benchmarks`.

No file in this directory is an external provider result.
The requests crossed each host's loopback HTTP interface and therefore exclude MIOSA's production edge, authentication, placement service, load balancer, and GitHub-runner network path.
