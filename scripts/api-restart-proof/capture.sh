#!/usr/bin/env bash
#
# What a machine that stopped answering was doing.
#
# Sourced by api-restart-proof.sh, which owns the globals this reads (STATE_ROOT, EVIDENCE, KEEP,
# created) and provides `record`.
#
# A machine that stopped answering is either gone or waiting behind a queue. A hosted machine is
# addressed over a unix socket under the runtime state root (`file_store`: the "machines"
# directory), so `ss -x` lists what is queued on that socket and `/proc/<pid>/wchan` names what its
# process is blocked in; together they separate the two without disturbing the host.

# Where hosted machines are addressed under the runtime state root, most specific first.
capture_state_roots() {
    if [[ -n "${STATE_ROOT}" ]]; then
        printf '%s\n' "${STATE_ROOT}/machines"
    fi
    if [[ -n "${XDG_STATE_HOME:-}" ]]; then
        printf '%s\n' "${XDG_STATE_HOME}/SOMA/v1/machines"
    fi
    printf '%s\n' "${HOME:-/root}/.local/state/SOMA/v1/machines" "/root/.local/state/SOMA/v1/machines"
}

# Writes capture-<instance>.txt: the socket, everything queued on it, and the host process.
capture_machine() {
    local instance="$1" directory socket="" pid
    local out="${EVIDENCE}/capture-${instance}.txt"
    while IFS= read -r directory; do
        # -S, not -f: this is a unix socket, and a regular-file test never matches one.
        if [[ -S "${directory}/${instance}.sock" ]]; then
            socket="${directory}/${instance}.sock"
            break
        fi
    done < <(capture_state_roots)
    if [[ -z "${socket}" ]]; then
        record "capture ${instance}: no ${instance}.sock under ${STATE_ROOT:-the default state root}"
        return
    fi
    # -p adds the owning process to each line, so one listing gives both the queue and the pid.
    # `|| true` matters: without it a host without iproute2, or a socket nothing holds, would fail
    # an assignment under `set -e` and take the whole proof down with it.
    pid="$(ss -x -a -p 2>/dev/null | grep -F "${socket}" | grep -o 'pid=[0-9]*' | head -1 \
        | cut -d= -f2 || true)"
    [[ -n "${pid}" ]] || pid="$(fuser "${socket}" 2>/dev/null | tr -d ' ' || true)"
    {
        printf 'socket %s\n' "${socket}"
        printf '%s\n' '-- unix sockets naming it (queued connections):'
        ss -x -a -p 2>/dev/null | grep -F "${socket}" || printf '(none)\n'
        if [[ -n "${pid}" ]]; then
            printf 'host pid %s\n' "${pid}"
            printf '%s' '-- wchan: '
            cat "/proc/${pid}/wchan" 2>/dev/null || printf 'unreadable'
            printf '\n'
        fi
    } >"${out}"
    record "capture ${instance}: ${socket}${pid:+ pid ${pid}}"
}

# Every machine that failed to answer, and every machine when SOMA_PROOF_KEEP=1, since a run that
# passes is the one that shows the queue was empty.
capture_machines() {
    local instance
    # `created` is the caller's array and shellcheck cannot see across the source boundary.
    # shellcheck disable=SC2154
    for instance in "${created[@]}"; do
        if grep -q "after ${instance}: ok" "${EVIDENCE}/proof.log"; then
            if [[ "${KEEP}" == "1" ]]; then
                capture_machine "${instance}"
            fi
        else
            capture_machine "${instance}"
        fi
    done
}
