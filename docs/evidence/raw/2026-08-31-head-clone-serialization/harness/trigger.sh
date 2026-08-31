#!/usr/bin/env bash
# What separates a fast cohort from a slow one on an idle host. Each cohort is bracketed by the
# device read counters and the XFS metadata counters, so a cohort's cost can be set against the
# work the filesystem actually did rather than against a guess.
set -uo pipefail
. /srv/soma/audit2/hostbusy.sh
N="${1:-150}"
BASE=/srv/soma/audit2
P=/srv/soma/mvp/target/release/examples/head_probe
R=$BASE/raw/trigger
mkdir -p "$R"
counters() {
    awk '$3 == "dm-1" { printf "%s %s ", $6, $10 }' /proc/diskstats
    awk '/^blk_map /{ printf "%s ", $2 } /^log /{ printf "%s ", $5 } /^ig /{ printf "%s", $3 }' \
        /proc/fs/xfs/stat
}
wait_quiet 12 600 || { printf 'gave up waiting for a quiet host\n'; exit 1; }
for ((i = 0; i < N; i++)); do
    read -r r0 t0 b0 f0 g0 <<< "$(counters)"
    line="$("$P" --template "$BASE/copies/0/overlay.raw" --dir "$BASE/heads/d0" \
        --threads 100 --cohorts 1 --gap-ms 0 --label trigger | grep '"cohort"')"
    read -r r1 t1 b1 f1 g1 <<< "$(counters)"
    printf '%s\n' "${line%\}}, \"sectors_read\":$((r1 - r0)), \"read_ms\":$((t1 - t0)),\
 \"blk_map\":$((b1 - b0)), \"log_force\":$((f1 - f0)), \"inode_recycle\":$((g1 - g0))}" \
        >> "$R/trigger.jsonl"
done
printf 'done %s\n' "$(date +%T)"
