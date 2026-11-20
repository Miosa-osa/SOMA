#!/bin/bash
#
# soma-desktop-session: starts, reports and stops the desktop session of a SOMA desktop machine.
#
# A SOMA guest has no init system and its Generation snapshot holds no running process, so the
# desktop is started after the machine is Ready, by one command run through exec:
#
#   soma-desktop-session start     start the session and wait until it is ready (prints JSON)
#   soma-desktop-session status    print the session's milestones as JSON
#   soma-desktop-session stop      end the session
#
# The guest agent kills every process when a command ends, except the ones the command names in
# /run/soma/survivors. A desktop cannot be named by process id, group or session: a terminal's
# shell and D-Bus activated services start sessions of their own. So the whole desktop runs in
# one cgroup, /soma-desktop, and the survivor line names that cgroup, the mechanism the guest
# agent already offers to a container engine.
#
# Every Instance restores the same captured bytes, so per-Instance identity is not this script's
# job: the guest agent writes /etc/machine-id for each Instance, and the image links the D-Bus
# machine id to it.

set -euo pipefail

USER_NAME="${SOMA_DESKTOP_USER:-ubuntu}"
STATE=/run/soma-desktop
CGROUP_ROOT=/sys/fs/cgroup
CGROUP_NAME=soma-desktop
SURVIVORS=/run/soma/survivors
SESSION=/usr/local/libexec/soma-desktop/session.sh
READY_TENTHS="${SOMA_DESKTOP_READY_TENTHS:-300}"

die() { printf 'soma-desktop-session: %s\n' "$1" >&2; exit 1; }

milestones_json() {
    local first=1
    printf '{'
    if [[ -f "$STATE/milestones" ]]; then
        while read -r name stamp; do
            ((first)) || printf ','
            first=0
            printf '"%s":%s' "$name" "$stamp"
        done <"$STATE/milestones"
    fi
    printf '}'
}

report() {
    printf '{"state":"%s","launcher_start_ns":%s,"milestones":%s}\n' \
        "$1" "$(cat "$STATE/launcher_start" 2>/dev/null || echo null)" "$(milestones_json)"
}

running() { [[ -f "$STATE/pid" ]] && kill -0 "$(cat "$STATE/pid")" 2>/dev/null; }

place_in_cgroup() {
    if ! mountpoint -q "$CGROUP_ROOT"; then
        mount -t cgroup2 -o nosuid,nodev,noexec cgroup2 "$CGROUP_ROOT" ||
            die "cgroup v2 is not available in this guest, so the desktop cannot survive its command"
    fi
    mkdir -p "$CGROUP_ROOT/$CGROUP_NAME"
    # Named before anything starts, so no process of the session can outrun the declaration.
    mkdir -p "$(dirname "$SURVIVORS")"
    grep -qx "cgroup:/$CGROUP_NAME" "$SURVIVORS" 2>/dev/null ||
        printf 'cgroup:/%s\n' "$CGROUP_NAME" >>"$SURVIVORS"
    printf '%s\n' "$$" >"$CGROUP_ROOT/$CGROUP_NAME/cgroup.procs"
}

start() {
    if running; then
        report running
        return 0
    fi
    local uid gid
    uid="$(id -u "$USER_NAME")" || die "no user $USER_NAME in this image"
    gid="$(id -g "$USER_NAME")"
    install -d -m 0755 "$STATE"
    : >"$STATE/milestones"
    date +%s%N >"$STATE/launcher_start"
    chown "$uid:$gid" "$STATE/milestones"
    install -d -m 0700 -o "$uid" -g "$gid" "/run/user/$uid" "$STATE/captures"
    install -d -m 1777 /tmp/.X11-unix
    place_in_cgroup
    # setsid -f: the session leaves this command's process group and keeps no descriptor of it,
    # so the command can end while the desktop runs on.
    setsid -f setpriv --reuid="$uid" --regid="$gid" --init-groups --reset-env \
        env XDG_RUNTIME_DIR="/run/user/$uid" SOMA_DESKTOP_STATE="$STATE" LANG=C.UTF-8 \
        "$SESSION" >"$STATE/session.log" 2>&1 </dev/null
    local i pid=""
    for ((i = 0; i < 50; i++)); do
        pid="$(pgrep -u "$uid" -f "^/bin/bash $SESSION" | head -1 || true)"
        [[ -n "$pid" ]] && break
        sleep 0.02
    done
    [[ -n "$pid" ]] || die "the session did not start; see $STATE/session.log"
    printf '%s\n' "$pid" >"$STATE/pid"
    for ((i = 0; i < READY_TENTHS; i++)); do
        if grep -q '^session_ready ' "$STATE/milestones"; then
            report ready
            return 0
        fi
        if grep -q '^failed_' "$STATE/milestones" || ! kill -0 "$pid" 2>/dev/null; then
            report failed
            tail -20 "$STATE/session.log" >&2
            return 1
        fi
        sleep 0.1
    done
    report timeout
    return 1
}

stop() {
    if running; then
        kill -TERM "$(cat "$STATE/pid")" 2>/dev/null || true
    fi
    if [[ -f "$CGROUP_ROOT/$CGROUP_NAME/cgroup.kill" ]]; then
        printf '1\n' >"$CGROUP_ROOT/$CGROUP_NAME/cgroup.kill"
    fi
    rm -f "$STATE/pid"
    report stopped
}

[[ "$(id -u)" == 0 ]] || die "run as root (it is run through exec)"
case "${1:-}" in
    start) start ;;
    status) if running; then report running; else report stopped; fi ;;
    stop) stop ;;
    *) die "usage: soma-desktop-session start|status|stop" ;;
esac
