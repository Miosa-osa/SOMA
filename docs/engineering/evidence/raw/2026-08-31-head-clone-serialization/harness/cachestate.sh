#!/usr/bin/env bash
# Whether cache state on the serialized metadata is what moves a cohort between its two modes,
# and whether the fanned out shape is protected from the same push. Warm and cold cohorts are
# interleaved within each round so host drift cannot separate them, and disk read counters are
# recorded either side of every cohort.
set -uo pipefail
. /srv/soma/audit2/hostbusy.sh
ROUNDS="${1:-15}"
LIMIT="${2:-12}"
BASE=/srv/soma/audit2
P=/srv/soma/mvp/target/release/examples/head_probe
R=$BASE/raw/cachestate
mkdir -p "$R"
many=""
for ((i = 0; i < 4; i++)); do many="${many:+$many,}$BASE/copies/$i/overlay.raw"; done
spread=""
for ((i = 0; i < 16; i++)); do spread="${spread:+$spread,}$BASE/heads/d$i"; done
reads() { awk '$3 == "dm-1" { print $6 }' /proc/diskstats; }
one() {
    local arm="$1" cache="$2" round="$3" tmpl dir before after
    case "$arm" in
        t1-d1) tmpl="$BASE/copies/0/overlay.raw"; dir="$BASE/heads/d0" ;;
        t4-d16) tmpl="$many"; dir="$spread" ;;
    esac
    if [[ "$cache" == cold ]]; then
        sync
        echo 3 | sudo tee /proc/sys/vm/drop_caches > /dev/null
        sleep 1
    fi
    before="$(reads)"
    "$P" --template "$tmpl" --dir "$dir" --threads 100 --cohorts 1 --gap-ms 0 \
        --label "$arm-$cache" --out "$R/$arm-$cache-r$(printf %02d "$round").jsonl" \
        | grep '"cohort"' >> "$R/$arm-$cache.cohorts"
    after="$(reads)"
    printf '{"arm":"%s","cache":"%s","round":%d,"sectors_read":%d}\n' \
        "$arm" "$cache" "$round" "$((after - before))" >> "$R/reads.jsonl"
}
for ((round = 0; round < ROUNDS; round++)); do
    wait_quiet "$LIMIT" 1800 || { printf 'gave up waiting for a quiet host\n'; exit 1; }
    for arm in t1-d1 t4-d16; do
        for cache in cold warm; do one "$arm" "$cache" "$round"; done
    done
done
printf 'done %s\n' "$(date +%T)"
