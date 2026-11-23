#!/usr/bin/env bash
# Queue depth against instability. If the clone segment is a queue of depth N on a serialized
# section, its cost and its cohort to cohort spread should both fall with N. Concurrencies are
# interleaved within each round so host drift hits them equally.
set -uo pipefail
. /srv/soma/audit2/hostbusy.sh
ROUNDS="${1:-18}"
LIMIT="${2:-12}"
R=/srv/soma/audit2/raw/depth
mkdir -p "$R"
for ((round = 0; round < ROUNDS; round++)); do
    wait_quiet "$LIMIT" 1800 || { printf 'gave up waiting for a quiet host\n'; exit 1; }
    for conc in 10 25 100; do
        busy="$(busy_pct 0.5)"
        out="$R/c$conc-$(printf %02d "$round").jsonl"
        bash /srv/soma/mvp-tti.sh /srv/soma/s-rw 1024 2048 "$conc" "$out" > /dev/null 2>&1
        printf '{"conc":%d,"round":%d,"busy_before":%s,"busy_after":%s}\n' \
            "$conc" "$round" "$busy" "$(busy_pct 0.5)" >> "$R/load.jsonl"
    done
done
printf 'done %s\n' "$(date +%T)"
