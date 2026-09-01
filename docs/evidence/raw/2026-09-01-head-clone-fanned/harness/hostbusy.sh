#!/usr/bin/env bash
# Instantaneous host busyness as a percentage of all cores over a short window.
#
# The one minute load average cannot gate this measurement: a cohort of a hundred sandboxes
# raises it by itself, so a run would refuse to continue because of its own last sample. A
# short /proc/stat window taken while this process is idle sees only the other tenants.
busy_pct() {
    local window="${1:-0.5}"
    local a b
    a="$(awk '/^cpu /{print $2+$3+$4+$6+$7+$8, $2+$3+$4+$5+$6+$7+$8}' /proc/stat)"
    sleep "$window"
    b="$(awk '/^cpu /{print $2+$3+$4+$6+$7+$8, $2+$3+$4+$5+$6+$7+$8}' /proc/stat)"
    awk -v a="$a" -v b="$b" 'BEGIN {
        split(a, x, " "); split(b, y, " ");
        total = y[2] - x[2];
        if (total <= 0) { print "-1"; exit }
        printf "%.2f", 100 * (y[1] - x[1]) / total
    }'
}

# Blocks until the host is quiet enough, or fails after the deadline.
wait_quiet() {
    local limit="${1:-12}" deadline="${2:-1800}" waited=0 now
    while true; do
        now="$(busy_pct 0.5)"
        awk -v a="$now" -v b="$limit" 'BEGIN { exit !(a >= 0 && a <= b) }' && return 0
        sleep 5
        waited=$((waited + 5))
        ((waited > deadline)) && return 1
    done
}
