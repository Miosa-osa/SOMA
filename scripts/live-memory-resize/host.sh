#!/usr/bin/env bash
#
# What live-memory-resize.sh does to a running machine and to the host process that holds it:
# ask the guest a question, resize it, and measure how much memory the host is really using.
#
# Sourced by live-memory-resize.sh, which owns the globals these read: INSTANCE, WORK, JUDGE,
# RESIZE_CMD, and the `soma` wrapper. Nothing here changes a stage's verdict by itself; each
# function reports a number or a one line outcome and the stage decides.

# The process that holds machine $1. The hosting process names its sandbox on its command line;
# when several match (a launcher still exiting, a watcher), the one holding the most memory is
# the machine's.
host_pid() {
    local instance="$1" state_root="$2" pid best="" best_rss=-1 rss
    for pid in $(pgrep -f -- "$instance" 2>/dev/null; pgrep -f -- "$state_root" 2>/dev/null); do
        [[ "$pid" != "$$" && "$pid" != "$BASHPID" && -r "/proc/$pid/smaps_rollup" ]] || continue
        rss="$(host_rss_kib "$pid")"
        (( rss > best_rss )) && { best="$pid"; best_rss="$rss"; }
    done
    printf '%s' "$best"
}

# Resident memory of a process in KiB: everything of it the host has actually backed with RAM,
# anonymous or shared, which is what falls when a plugged block is given back.
host_rss_kib() {
    awk '/^Rss:/ { print $2; found = 1 } END { if (!found) print 0 }' \
        "/proc/$1/smaps_rollup" 2>/dev/null || printf '0\n'
}

# Runs one command in the guest; prints its stdout, fails with the reason otherwise.
guest_exec() {
    local out="$WORK/exec-$RANDOM$RANDOM.json"
    # shellcheck disable=SC2153 # INSTANCE is the sourcing script's global, set once launched.
    soma machine exec --instance-id "$INSTANCE" -- "$@" >"$out" 2>"$out.err" \
        || { tail -c 200 "$out.err"; return 1; }
    python3 "$JUDGE" stdout "$out"
}

guest_memtotal_kib() {
    local text
    text="$(guest_exec /bin/cat /proc/meminfo)" || { printf '%s' "$text"; return 1; }
    python3 "$JUDGE" meminfo MemTotal <<<"$text"
}

# Resizes the machine to a total of $1 MiB and prints "<state> <plugged> <total> <booted> <ceiling>".
resize_to() {
    local mib="$1" out="$WORK/resize-$RANDOM$RANDOM.json" command
    if [[ -n "$RESIZE_CMD" ]]; then
        command="${RESIZE_CMD//\{id\}/$INSTANCE}"
        command="${command//\{mib\}/$mib}"
        # The template is the operator's own command line, run as written.
        eval "$command" >"$out" 2>"$out.err" || { tail -c 200 "$out.err"; return 1; }
    else
        soma machine resize --instance-id "$INSTANCE" --memory-mib "$mib" >"$out" 2>"$out.err" \
            || { tail -c 200 "$out.err" "$out"; return 1; }
    fi
    python3 "$JUDGE" memory "$out"
}

# Polls the guest's MemTotal until it is at least (or at most) $1 KiB, for up to $2 seconds.
wait_memtotal_at_least() { wait_memtotal "$1" "$2" ge; }
wait_memtotal_at_most() { wait_memtotal "$1" "$2" le; }

wait_memtotal() {
    local want="$1" seconds="$2" relation="$3" waited=0 now
    while (( waited <= seconds )); do
        now="$(guest_memtotal_kib 2>/dev/null)" && [[ "$now" =~ ^[0-9]+$ ]] && {
            [[ "$relation" == ge ]] && (( now >= want )) && return 0
            [[ "$relation" == le ]] && (( now <= want )) && return 0
        }
        sleep 1
        waited=$(( waited + 1 ))
    done
    return 1
}

# The highest resident size of $1 seen every half second while background job $2 runs.
sample_peak_rss_kib() {
    local pid="$1" worker="$2" peak=0 now
    while kill -0 "$worker" 2>/dev/null; do
        now="$(host_rss_kib "$pid")"
        (( now > peak )) && peak="$now"
        sleep 0.5
    done
    printf '%s' "$peak"
}

# Waits up to $3 seconds for the resident size of $1 to fall to $2 KiB; prints the last reading.
wait_rss_below() {
    local pid="$1" target="$2" seconds="$3" waited=0 now
    while :; do
        now="$(host_rss_kib "$pid")"
        (( now <= target || waited >= seconds )) && break
        sleep 1
        waited=$(( waited + 1 ))
    done
    printf '%s' "$now"
}
