#!/usr/bin/env bash
# One cohort of restored sandboxes, with the page-in state of the ones that can be read.
#
# `MODE=probe` runs the configuration's real probe program and reports the receipt segments,
# which is the figure that matters. `MODE=sleep` runs a sleeping command instead so the monitor
# process is still alive when `/proc/<pid>/smaps` is read, which is the only way to attribute
# faults to the captured memory mapping rather than to the process as a whole.
set -uo pipefail
STORE="$1"; MEM="$2"; DISK="$3"; IMAGE="$4"; CONC="$5"; MODE="$6"; LABEL="$7"
shift 7
PROG=("$@")
REPO=/srv/soma/pagein/SOMA
OUT=/srv/soma/pagein/out
mkdir -p "$OUT/$LABEL" /srv/soma/pagein/heads
cd "$REPO"
export SOMA_GENERATION_STORE="$STORE"
export SOMA_HEAD_DIR=/srv/soma/pagein/heads
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1
rm -f "$OUT/$LABEL"/*

if [[ "$MODE" == sleep ]]; then PROG=(/bin/sleep 3); fi
BAR="$OUT/$LABEL/.barrier"

one() {
    while [[ ! -f "$BAR" ]]; do :; done
    ./target/release/soma --format json --backend kvm run --memory-mib "$MEM" \
        --storage-mib "$DISK" --timeout-ms 20000 "$IMAGE" -- "${PROG[@]}" \
        > "$OUT/$LABEL/$1.json" 2>/dev/null &
    local pid=$!
    if [[ "$MODE" == sleep && "$1" -le 10 ]]; then
        sleep 0.5
        python3 /srv/soma/pagein/smaps.py "$pid" $((MEM * 1024)) "$LABEL-$1" \
            > "$OUT/$LABEL/$1.smaps" 2>/dev/null
    fi
    wait "$pid"
}

for i in $(seq 1 "$CONC"); do one "$i" & done
sleep 1
: > "$BAR"
wait
python3 /srv/soma/pagein/summarise.py "$OUT/$LABEL" "$LABEL" "$MEM" "$CONC" "$MODE"
