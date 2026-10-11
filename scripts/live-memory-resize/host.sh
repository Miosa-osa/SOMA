#!/usr/bin/env bash
#
# What live-memory-resize.sh does to a running machine and to the host process that holds it:
# ask the guest a question, resize it, and measure how much memory the host is really using.
#
# Sourced by live-memory-resize.sh, which owns the globals these read: INSTANCE, WORK, JUDGE,
# RESIZE_CMD, and the `soma` wrapper. Nothing here changes a stage's verdict by itself; each
# function reports a number or a one line outcome and the stage decides.

# The process that holds the machine, taken from what a resize says. The machine host is a plain
# `soma machine-host` that carries no identity on its command line, so a process search cannot
# find it; asking for the CPU count the machine already has changes nothing and returns host_pid.
host_pid() {
    local cpus="$1" out="$WORK/probe-$RANDOM$RANDOM.json"
    soma machine resize --instance-id "$INSTANCE" --cpu "$cpus" >"$out" 2>"$out.err" || true
    python3 "$JUDGE" hostpid "$out"
}

# Memory the host has spent on the machine, in KiB. A jailed machine's VMM runs in its own
# cgroup leaf, `soma-<instance>` under $SOMA_JAIL_CGROUP_ROOT, whose memory.current is the whole
# cost. A resident machine lives inside the host process, whose resident size is the cost.
host_rss_kib() {
    local leaf="${SOMA_JAIL_CGROUP_ROOT:-}/soma-${INSTANCE:-}/memory.current"
    if [[ -n "${SOMA_JAIL_CGROUP_ROOT:-}" && -r "$leaf" ]]; then
        awk '{ print int($1 / 1024) }' "$leaf"
        return
    fi
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
    local mib="$1" out="$WORK/resize-$RANDOM$RANDOM.json" command status=0
    if [[ -n "$RESIZE_CMD" ]]; then
        command="${RESIZE_CMD//\{id\}/$INSTANCE}"
        command="${command//\{mib\}/$mib}"
        # The template is the operator's own command line, run as written.
        eval "$command" >"$out" 2>"$out.err" || status=$?
    else
        soma machine resize --instance-id "$INSTANCE" --memory-mib "$mib" >"$out" 2>"$out.err" \
            || status=$?
    fi
    # 0 applied it, 69 applied nothing but still reports the numbers that are true now. Anything
    # else (66 not found, 74 the host did not answer) has no answer to read.
    case "$status" in
        0|69) python3 "$JUDGE" memory "$out" ;;
        *) printf 'exit %s: ' "$status"; tail -c 200 "$out.err" "$out"; return 1 ;;
    esac
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
