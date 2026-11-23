#!/usr/bin/env bash
# Alternating writable and read-only cohorts through the existing cohort harness. This host is
# shared with other agents, so each round waits for a quiet host, both the busy percentage and
# the load average are recorded on both sides of every cohort, and a cohort taken while other
# tenants were working is discarded during analysis rather than averaged through.
set -uo pipefail
. /srv/soma/audit2/hostbusy.sh
N="${1:-30}"
TAG="${2:-gated}"
LIMIT="${3:-12}"
R=/srv/soma/audit2/raw/$TAG
mkdir -p "$R"
cohort() {
    local arm="$1" store="$2" disk="$3" index="$4"
    local out="$R/$arm-$(printf %02d "$index").jsonl"
    local busy load
    busy="$(busy_pct 0.5)"
    load="$(cut -d' ' -f1 /proc/loadavg)"
    bash /srv/soma/mvp-tti.sh "$store" 1024 "$disk" 100 "$out" > /dev/null 2>&1
    printf '{"arm":"%s","cohort":%d,"busy_before":%s,"load_before":%s,"busy_after":%s,"at":"%s"}\n' \
        "$arm" "$index" "$busy" "$load" "$(busy_pct 0.5)" "$(date +%T)" >> "$R/load.jsonl"
}
for ((i = 0; i < N; i++)); do
    wait_quiet "$LIMIT" 1800 || { printf 'gave up waiting for a quiet host\n'; exit 1; }
    cohort rw /srv/soma/s-rw 2048 "$i"
    cohort ro /srv/soma/s-ro 0 "$i"
done
printf 'done %s\n' "$(date +%T)"
