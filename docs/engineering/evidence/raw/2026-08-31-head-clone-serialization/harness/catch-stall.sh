#!/usr/bin/env bash
# Dumps kernel stacks the moment most of the probe's threads are blocked, so what they are
# waiting on is read off the kernel rather than inferred from timings. One `ps` call per poll:
# a per thread loop is far too slow to land inside a cohort that lasts twenty milliseconds.
set -uo pipefail
OUT="$1"; THRESH="${2:-25}"; MAX="${3:-6}"
: > "$OUT"
caught=0
while (( caught < MAX )); do
    pid="$(pgrep -x head_probe | head -1)"
    if [[ -z "$pid" ]]; then sleep 0.05; continue; fi
    d="$(ps -L -p "$pid" -o s= 2>/dev/null | grep -c D)"
    if (( d >= THRESH )); then
        caught=$((caught + 1))
        {
            echo "=== catch $caught at $(date +%H:%M:%S.%N) blocked=$d"
            echo "--- probe thread stacks, grouped"
            cat /proc/"$pid"/task/*/stack 2>/dev/null \
                | awk '/entry_SYSCALL|^$/ { if (s != "") { c[s]++; s = "" }; next }
                       { s = s $0 "\n" }
                       END { for (k in c) printf "COUNT %d\n%s\n", c[k], k }' \
                | head -80
            echo "--- other tasks in uninterruptible sleep"
            ps -eo pid,stat,comm | awk '$2 ~ /D/ && $3 != "head_probe" { print $1, $3 }' \
                | while read -r p c; do
                    echo "[$p $c]"; head -8 /proc/"$p"/stack 2>/dev/null
                done
            echo "--- xfs stat"
            grep -E '^(log|push_ail|trans|ig) ' /proc/fs/xfs/stat
        } >> "$OUT" 2>&1
        sleep 0.2
    fi
done
