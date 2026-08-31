#!/usr/bin/env bash
# The whole before-and-after matrix: two Generations, two concurrencies, two modes.
#
# `probe` cohorts carry the receipt segments and the time to first command; one sandbox is
# not a distribution, so concurrency one is repeated nine times and concurrency a hundred
# three times. `sleep` cohorts carry the page-in state, because only a sandbox that is still
# alive can be read from `/proc`. Both modes run the same restore, so the fault counts and
# the segments describe the same work even though no single sandbox reports both.
set -uo pipefail
TAG="${1:-before}"
OUT=/srv/soma/pagein/results-$TAG.jsonl
: > "$OUT"
cd /srv/soma/pagein

CASES=(
  "bb   /srv/soma/sweep/bb-1024-10240   1024 10240 busybox:stable-musl /bin/busybox --help"
  "node /srv/soma/sweep/node-1024-10240 1024 10240 node:22 /usr/local/bin/node --version"
)

run() {
    local label="$1"; shift
    bash measure.sh "$@" "$label" "$PROG" "$ARG" | tee -a "$OUT"
}

for row in "${CASES[@]}"; do
    read -r name store mem disk image PROG ARG <<< "$row"
    bash measure.sh "$store" "$mem" "$disk" "$image" 1 probe "$name-warm" "$PROG" "$ARG" >/dev/null
    for rep in $(seq 1 9); do
        run "$TAG-$name-c1-probe-$rep" "$store" "$mem" "$disk" "$image" 1 probe
    done
    run "$TAG-$name-c1-sleep-1" "$store" "$mem" "$disk" "$image" 1 sleep
    for rep in 1 2 3; do
        run "$TAG-$name-c100-probe-$rep" "$store" "$mem" "$disk" "$image" 100 probe
    done
    run "$TAG-$name-c100-sleep-1" "$store" "$mem" "$disk" "$image" 100 sleep
done
echo "=== $TAG matrix complete ==="
