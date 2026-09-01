#!/usr/bin/env bash
# Three cohorts per arm, alternating, because the host carries other agents' work and a
# block of one arm followed by a block of the other would compare two different hosts.
set -euo pipefail
cd /srv/soma/durable-perf
for r in 4 5 6; do
    RUNS="$r" ./bench.sh baseline /srv/soma/durable-perf/SOMA /srv/soma/dp
    RUNS="$r" ./bench.sh after /srv/soma/durable-perf/SOMA-after /srv/soma/dp
done
echo interleave-done
