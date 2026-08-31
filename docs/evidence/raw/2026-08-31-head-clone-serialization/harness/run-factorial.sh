#!/usr/bin/env bash
# Builds the independent template copies and immediately runs the crossed design, one cohort
# at a time, round robin over the four arms so host drift hits all four equally. Every cohort
# carries the one minute load average it was taken under and each round waits for a quiet host.
set -uo pipefail
ROUNDS="${1:-25}"
LIMIT="${2:-5}"
COPIES="${3:-4}"
DIRS="${4:-16}"
BASE=/srv/soma/audit2
P=/srv/soma/mvp/target/release/examples/head_probe
T=/srv/soma/s-rw/ref-b1bfde16c842956f7bdb469eb109b89bbdccd284d6f8ff5a6a5af448f8e0bb25/snapshot/overlay.raw
R=$BASE/raw/factorial
mkdir -p "$R"
many=""
for ((i = 0; i < COPIES; i++)); do
    mkdir -p "$BASE/copies/$i"
    [[ -f "$BASE/copies/$i/overlay.raw" ]] || cp --reflink=never "$T" "$BASE/copies/$i/overlay.raw"
    many="${many:+$many,}$BASE/copies/$i/overlay.raw"
done
sync
for ((i = 0; i < COPIES; i++)); do
    xfs_bmap -vp "$BASE/copies/$i/overlay.raw" | sed -n 3p >> "$R/copy-extents.txt"
done
spread=""
for ((i = 0; i < DIRS; i++)); do spread="${spread:+$spread,}$BASE/heads/d$i"; done
wait_quiet() {
    local waited=0 now
    while true; do
        now="$(cut -d' ' -f1 /proc/loadavg)"
        awk -v a="$now" -v b="$LIMIT" 'BEGIN { exit !(a <= b) }' && return 0
        sleep 10
        waited=$((waited + 10))
        ((waited > 1800)) && return 1
    done
}
for ((round = 0; round < ROUNDS; round++)); do
    wait_quiet || { printf 'gave up waiting for a quiet host\n'; exit 1; }
    for arm in t1-d1 t4-d1 t1-d16 t4-d16; do
        case "$arm" in
            t1-d1) tmpl="$BASE/copies/0/overlay.raw"; dir="$BASE/heads/d0" ;;
            t4-d1) tmpl="$many"; dir="$BASE/heads/d0" ;;
            t1-d16) tmpl="$BASE/copies/0/overlay.raw"; dir="$spread" ;;
            t4-d16) tmpl="$many"; dir="$spread" ;;
        esac
        "$P" --template "$tmpl" --dir "$dir" --threads 100 --cohorts 1 --gap-ms 0 \
            --label "$arm" --out "$R/$arm-r$(printf %02d "$round").jsonl" \
            | grep '"cohort"' >> "$R/$arm.cohorts"
    done
done
printf 'done %s\n' "$(date +%T)"
