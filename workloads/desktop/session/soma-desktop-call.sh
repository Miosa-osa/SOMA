#!/bin/bash
#
# soma-desktop-call: runs one cua-driver command as the desktop user, against the session's driver.
#
# Commands arrive through exec as root. cua-driver must run as the desktop user (its socket is
# owner-only and AT-SPI lives on that user's session bus), so this drops to that user with the
# session's environment and nothing else, then runs the driver's own command line:
#
#   soma-desktop-call doctor
#   soma-desktop-call call get_desktop_state '{"max_image_dimension":0}'
#
# A screenshot can be written straight to a file with the driver's own option:
#   soma-desktop-call call get_desktop_state '{}' --screenshot-out-file /tmp/desktop.png

set -euo pipefail

USER_NAME="${SOMA_DESKTOP_USER:-ubuntu}"
uid="$(id -u "$USER_NAME")"
gid="$(id -g "$USER_NAME")"

if [[ "$(id -u)" == "$uid" ]]; then
    exec env DISPLAY=:0 XDG_RUNTIME_DIR="/run/user/$uid" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$uid/bus" cua-driver "$@"
fi
[[ "$(id -u)" == 0 ]] || { printf 'soma-desktop-call: run as root or as %s\n' "$USER_NAME" >&2; exit 1; }
exec setpriv --reuid="$uid" --regid="$gid" --init-groups --reset-env \
    env DISPLAY=:0 XDG_RUNTIME_DIR="/run/user/$uid" LANG=C.UTF-8 \
    DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$uid/bus" cua-driver "$@"
