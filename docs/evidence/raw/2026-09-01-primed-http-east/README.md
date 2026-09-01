# Raw primed-machine exact HTTP shards

This directory retains three synchronized one-hundred-sandbox cohorts from the final prepared-machine qualification binary.
Every cohort used a 40/20/40 placement across host03, host04, and host10 with one shared Unix release epoch.
Every sample starts before public create, stops after a successful `/usr/local/bin/node -v`, and records cleanup outside the timing boundary.
The final binary SHA-256 is `fbeb7229640c56876799196752daf2ed787e2ca545b38c4fa9aa5105324bff90`.

| Cohort | Median | p95 | p99 | Raw maximum | Command and cleanup success | Qualification |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| 1 | 60.01 ms | 69.27 ms | 69.85 ms | 73.59 ms | 100/100 | Clean capability evidence |
| 2 | 63.78 ms | 70.75 ms | 71.04 ms | 76.66 ms | 100/100 | Clean capability evidence |
| 3 | 68.17 ms | 1,168.84 ms | 1,173.48 ms | 1,175.58 ms | 100/100 | Contended resilience evidence only |

All receipts report `prepared_worker`.
After every cohort each API returned to 64 child processes with zero zombies.

The third cohort is retained rather than discarded because it demonstrates the effect of unrelated shared-host work.
All forty host10 samples slowed together while its internal median admission rose from approximately 6 ms to 133 ms, Ready rose from approximately 16 ms to 720 ms, and first command rose from approximately 23 ms to 391 ms.
The host simultaneously showed active encrypted-device `kcryptd` workers and high MIOSA control-plane CPU use.
This cohort proves successful operation under contention but cannot support a clean latency claim.

The shard files are the original JSON documents written create-exclusively on each host.
The combined statistics are deterministic recomputations using the inspected ComputeSDK five-percent trim, arithmetic median, and nearest-rank p95 and p99.
The inspected upstream benchmark revision remains `46dea652fcc372e5acea0c9f372613d86b4b6bab`.
These are host-loopback qualification results and exclude the MIOSA edge, external authentication, fleet placement, load balancing, and GitHub-runner network path.
