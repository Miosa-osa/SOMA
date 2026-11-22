#!/bin/bash
#
# The desktop session itself, run as the desktop user by soma-desktop-session.
#
# Order matters and each step waits for the one before it:
#   1. the session D-Bus at $XDG_RUNTIME_DIR/bus, the address a login session would have, so any
#      later process (cua-driver, a command run through exec) finds it without being told;
#   2. Xorg on :0 with the dummy video driver, the screen, with no VNC server in it;
#   3. the accessibility bus, which cua-driver reads the AT-SPI tree from;
#   4. Openbox, the window manager;
#   5. cua-driver serve, the agent's hands and eyes, on its owner-only Unix socket.
#
# Every milestone is written to $SOMA_DESKTOP_STATE/milestones as "<name> <unix ns>", which is how
# the launcher reports readiness and how a measurement names its stages.
#
# Nothing here forks itself into the background: every process stays a child of this shell, in
# the cgroup the launcher placed it in, so the guest agent's sweep spares the whole tree.

set -u

STATE="${SOMA_DESKTOP_STATE:?SOMA_DESKTOP_STATE is set by soma-desktop-session}"
MANIFEST=/etc/soma-desktop/cua-capabilities.yaml
BUS="unix:path=${XDG_RUNTIME_DIR:?}/bus"

mark() { printf '%s %s\n' "$1" "$(date +%s%N)" >>"$STATE/milestones"; }
log() { printf '[session] %s %s\n' "$(date +%T.%3N)" "$*"; }

# Waits up to $2 tenths of a second for the command in $3... to succeed.
wait_for() {
    local name="$1" tenths="$2"
    shift 2
    local i
    for ((i = 0; i < tenths; i++)); do
        if "$@" >/dev/null 2>&1; then
            mark "$name"
            return 0
        fi
        sleep 0.1
    done
    log "timed out waiting for $name"
    mark "failed_$name"
    return 1
}

port_open() { (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null; }

# True when every named top-level window is viewable on the display.
viewable() {
    local name
    for name in "$@"; do
        xwininfo -display :0 -name "$name" 2>/dev/null | grep -q 'Map State: IsViewable' || return 1
    done
}

trap 'kill 0' TERM INT

mark session_start
dbus-daemon --session --address="$BUS" --nofork --nopidfile &
export DBUS_SESSION_BUS_ADDRESS="$BUS"
wait_for session_bus 50 test -S "$XDG_RUNTIME_DIR/bus" || exit 1

# Xorg with the dummy driver is the screen: a frame buffer in memory, no device, no VNC. It runs as
# the desktop user, so it is started directly (not through the setuid wrapper) with its log in the
# session state, and it opens no virtual terminal.
/usr/lib/xorg/Xorg :0 \
    -config /etc/soma-desktop/xorg.conf \
    -logfile "$STATE/logs/Xorg.log" \
    -nolisten tcp \
    -noreset \
    -novtswitch \
    -sharevts \
    -keeptty \
    +extension RANDR +extension RENDER +extension XTEST &
wait_for x_socket 150 test -S /tmp/.X11-unix/X0 || exit 1
export DISPLAY=:0

# Accessibility on for every toolkit the desktop may run: GTK 3 loads its AT-SPI bridge whenever
# the a11y bus answers, Qt and older GNOME libraries need to be told.
export QT_ACCESSIBILITY=1 QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1 GNOME_ACCESSIBILITY=1
/usr/libexec/at-spi-bus-launcher --launch-immediately &
wait_for a11y_bus 50 dbus-send --session --print-reply --dest=org.a11y.Bus /org/a11y/bus org.a11y.Bus.GetAddress || exit 1

openbox --sm-disable --config-file /etc/soma-desktop/openbox.xml &
# The desktop is up once Openbox has claimed the screen: it sets _NET_SUPPORTING_WM_CHECK on the
# root window when it manages it. The background is painted after that.
wait_for window_manager 200 pgrep -u "$(id -u)" -x openbox || exit 1
wait_for desktop_mapped 300 sh -c 'xprop -root _NET_SUPPORTING_WM_CHECK | grep -q "window id"' || exit 1
# The MIOSA look: a dark gradient behind everything, the top bar and the dock. Each is cosmetic, so
# a part that fails to start leaves a working desktop rather than no desktop.
hsetroot -add '#151a2a' -add '#0b0d12' -gradient 160 >/dev/null 2>&1 \
    || xsetroot -solid '#0b0d12' 2>/dev/null || true
xset s off s noblank -dpms 2>/dev/null || true
tint2 -c /etc/soma-desktop/tint2/bar.tint2rc >"$STATE/logs/bar.log" 2>&1 &
tint2 -c /etc/soma-desktop/tint2/dock.tint2rc >"$STATE/logs/dock.log" 2>&1 &

cua-driver serve \
    --permission-mode standard \
    --capability-manifest "$MANIFEST" \
    --approve-capability-manifest &
wait_for driver_socket 150 test -S "$HOME/.cache/cua-driver/cua-driver.sock" || exit 1
mark session_ready
log "desktop ready on :0"

# The session lives as long as its X server does.
wait
