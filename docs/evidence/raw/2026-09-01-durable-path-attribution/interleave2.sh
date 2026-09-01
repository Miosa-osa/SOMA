#!/usr/bin/env bash
# The same two arms with the order reversed, and a settle between cohorts. In the first block
# every "after" cohort ran immediately behind a "baseline" one, so it inherited a filesystem
# still freeing a hundred overlay heads. This block tests whether that order, not the code,
# produced the difference.
set -euo pipefail
cd /srv/soma/durable-perf
settle() { sync; sleep 45; }
for r in 7 8 9; do
    settle; RUNS="$r" ./bench.sh after /srv/soma/durable-perf/SOMA-after /srv/soma/dp
    settle; RUNS="$r" ./bench.sh baseline /srv/soma/durable-perf/SOMA /srv/soma/dp
done
echo interleave2-done
