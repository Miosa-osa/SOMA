#!/usr/bin/env bash
# One cohort of restored launches, both the receipt TTI and the ready segment, to one file.
#
# The launch page slot removal arm is chosen by the caller s environment, so both arms are the
# same binary and the same Generation and differ only in when a memory slot leaves the VM.
set -uo pipefail
CONC="${1:?concurrency}"; LABEL="${2:?label}"
REPO=/srv/soma/pagein/SOMA
OUT=/srv/soma/ept/cohorts
mkdir -p "$OUT" /srv/soma/ept/heads
cd "$REPO"
export SOMA_GENERATION_STORE=/srv/soma/sweep/node-1024-10240
export SOMA_HEAD_DIR=/srv/soma/ept/heads
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1
unset SOMA_KVM_TIMELINE
RAW="$(mktemp -d)"; BAR="$RAW/barrier"
REPS="${REPS:-1}"
one() {
    while [[ ! -f "$BAR" ]]; do :; done
    ./target/release/soma --format json --backend kvm run --memory-mib 1024 \
        --storage-mib 10240 node:22 -- /usr/local/bin/node --version \
        > "$RAW/$1.json" 2>/dev/null
}
head -1 /proc/stat > "$RAW/stat-before"
for rep in $(seq 1 "$REPS"); do
    rm -f "$BAR"
    for i in $(seq 1 "$CONC"); do one "$rep-$i" & done
    sleep 0.3
    : > "$BAR"
    wait
done
head -1 /proc/stat > "$RAW/stat-after"
python3 /srv/soma/ept/cohort.py "$RAW" "$OUT/$LABEL.json" "$CONC" "$LABEL"
rm -rf "$RAW"
