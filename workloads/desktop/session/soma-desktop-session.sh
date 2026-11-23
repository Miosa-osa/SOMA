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
#   soma-desktop-session viewer start|stop
#                                  start or end the human viewer (Sunshine + moonlight-web on the
#                                  viewer port), which runs only while someone watches
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
VIEWER=/usr/local/libexec/soma-desktop/viewer.sh
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

# Running means the session AND its screen are alive: a live session with no screen server is a
# black desktop, and reporting it as running hid exactly that (2026-11-23).
screen_alive() { [[ -S /tmp/.X11-unix/X0 ]] && pgrep -x 'Xvfb|Xorg|Xkasmvnc' >/dev/null 2>&1; }
running() { [[ -f "$STATE/pid" ]] && kill -0 "$(cat "$STATE/pid")" 2>/dev/null && screen_alive; }
viewer_running() { [[ -f "$STATE/viewer.pid" ]] && kill -0 "$(cat "$STATE/viewer.pid")" 2>/dev/null; }

# udev, so Xorg adds the pointer and keyboard the viewer creates through /dev/uinput. A SOMA guest
# has no init system to start it; it runs in the desktop's cgroup and lives as long as the desktop.
# /dev/uinput is opened by Sunshine as the desktop user, so it belongs to the input group.
start_udev() {
    pgrep -x systemd-udevd >/dev/null 2>&1 && return 0
    [[ -x /lib/systemd/systemd-udevd ]] || return 0
    /lib/systemd/systemd-udevd --daemon >/dev/null 2>&1 || return 0
    udevadm trigger --action=add >/dev/null 2>&1 || true
    udevadm settle --timeout=5 >/dev/null 2>&1 || true
    if [[ -c /dev/uinput ]]; then
        chgrp input /dev/uinput && chmod 0660 /dev/uinput
    fi
}

# Runs a program as the desktop user, detached from this command, in the desktop's cgroup.
as_desktop_user() {
    local uid="$1" gid="$2" log="$3"
    shift 3
    setsid -f setpriv --reuid="$uid" --regid="$gid" --init-groups --reset-env \
        env HOME="/home/$USER_NAME" XDG_RUNTIME_DIR="/run/user/$uid" SOMA_DESKTOP_STATE="$STATE" LANG=C.UTF-8 \
        "$@" >"$log" 2>&1 </dev/null
}

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
    # A session whose screen died: end what is left of it before starting a fresh one.
    if [[ -f "$STATE/pid" ]] && kill -0 "$(cat "$STATE/pid")" 2>/dev/null; then
        stop >/dev/null
    fi
    local uid gid
    uid="$(id -u "$USER_NAME")" || die "no user $USER_NAME in this image"
    gid="$(id -g "$USER_NAME")"
    install -d -m 0755 "$STATE"
    : >"$STATE/milestones"
    date +%s%N >"$STATE/launcher_start"
    chown "$uid:$gid" "$STATE/milestones"
    # The desktop user's own logs (Xorg, the viewer): the state directory itself is root's.
    install -d -m 0700 -o "$uid" -g "$gid" "/run/user/$uid" "$STATE/captures" "$STATE/logs"
    install -d -m 1777 /tmp/.X11-unix
    place_in_cgroup
    start_udev
    # setsid -f: the session leaves this command's process group and keeps no descriptor of it,
    # so the command can end while the desktop runs on.
    as_desktop_user "$uid" "$gid" "$STATE/session.log" "$SESSION"
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

viewer_start() {
    running || die "the desktop is not running; start it first"
    if viewer_running; then
        report viewer_running
        return 0
    fi
    local uid gid i pid=""
    uid="$(id -u "$USER_NAME")"
    gid="$(id -g "$USER_NAME")"
    place_in_cgroup
    sed -i '/^viewer_\|^failed_viewer_/d' "$STATE/milestones"
    as_desktop_user "$uid" "$gid" "$STATE/viewer.log" "$VIEWER"
    for ((i = 0; i < 50; i++)); do
        pid="$(pgrep -u "$uid" -f "^/bin/bash $VIEWER" | head -1 || true)"
        [[ -n "$pid" ]] && break
        sleep 0.02
    done
    [[ -n "$pid" ]] || die "the viewer did not start; see $STATE/viewer.log"
    printf '%s\n' "$pid" >"$STATE/viewer.pid"
    for ((i = 0; i < READY_TENTHS; i++)); do
        if grep -q '^viewer_ready ' "$STATE/milestones"; then
            report viewer_ready
            return 0
        fi
        if grep -q '^failed_viewer' "$STATE/milestones" || ! kill -0 "$pid" 2>/dev/null; then
            report viewer_failed
            tail -20 "$STATE/viewer.log" "$STATE/logs/sunshine.log" "$STATE/logs/moonlight-web.log" >&2 2>/dev/null
            return 1
        fi
        sleep 0.1
    done
    report viewer_timeout
    return 1
}

viewer_stop() {
    if viewer_running; then
        kill -TERM "$(cat "$STATE/viewer.pid")" 2>/dev/null || true
    fi
    rm -f "$STATE/viewer.pid"
    report viewer_stopped
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
    viewer)
        case "${2:-}" in
            start) viewer_start ;;
            stop) viewer_stop ;;
            *) die "usage: soma-desktop-session viewer start|stop" ;;
        esac
        ;;
    *) die "usage: soma-desktop-session start|status|stop|viewer start|viewer stop" ;;
esac
