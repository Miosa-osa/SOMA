#!/bin/bash
#
# The desktop session itself, run as the desktop user by soma-desktop-session.
#
# Order matters and each step waits for the one before it:
#   1. the session D-Bus at $XDG_RUNTIME_DIR/bus, the address a login session would have, so any
#      later process (cua-driver, a command run through exec) finds it without being told;
#   2. Xkasmvnc on :0, which is the X server and the VNC server in one process, with the same
#      flags the Firecracker desktop uses (WebSocket on 6080, authentication at the platform);
#   3. the accessibility bus, which cua-driver reads the AT-SPI tree from;
#   4. Xfce;
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
GEOMETRY="${SOMA_DESKTOP_GEOMETRY:-1920x1080}"
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

# -publicIP: without it KasmVNC 1.4 asks public STUN servers for the machine's address before it
# opens any port, and exits without a word when none answers, so a guest with no egress never got
# a desktop. The address only feeds KasmVNC's optional UDP transport, which the platform's viewer
# does not use; naming loopback keeps the guest from calling out and the viewer on WebSocket.
# -httpd: the viewer port also serves KasmVNC's own web client, so a browser pointed at 6080
# (through the tunnel) is a complete viewer; the platform's relay uses only its /websockify path.
Xkasmvnc :0 \
    -publicIP 127.0.0.1 \
    -httpd /usr/share/kasmvnc/www \
    -geometry "$GEOMETRY" \
    -depth 24 \
    -websocketPort 6080 \
    -SecurityTypes None \
    -disableBasicAuth \
    -interface 0.0.0.0 \
    -AlwaysShared \
    -AcceptSetDesktopSize=0 \
    -AcceptKeyEvents \
    -AcceptPointerEvents \
    -SendCutText \
    -AcceptCutText \
    -PreferBandwidth \
    -DynamicQualityMin=6 \
    -DynamicQualityMax=9 \
    -FrameRate=45 \
    -BlacklistThreshold=0 \
    -FreeKeyMappings &
wait_for x_socket 150 test -S /tmp/.X11-unix/X0 || exit 1
wait_for viewer_port 150 port_open 6080 || exit 1
export DISPLAY=:0

# Accessibility on for every toolkit the desktop may run: GTK 3 loads its AT-SPI bridge whenever
# the a11y bus answers, Qt and older GNOME libraries need to be told.
export QT_ACCESSIBILITY=1 QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1 GNOME_ACCESSIBILITY=1
/usr/libexec/at-spi-bus-launcher --launch-immediately &
wait_for a11y_bus 50 dbus-send --session --print-reply --dest=org.a11y.Bus /org/a11y/bus org.a11y.Bus.GetAddress || exit 1

startxfce4 &
wait_for window_manager 200 pgrep -u "$(id -u)" -x xfwm4 || exit 1
# A running panel process is not a painted desktop: its window maps a moment later. The desktop
# counts as up once the panel and the desktop background are both viewable on :0.
wait_for desktop_mapped 300 viewable xfce4-panel Desktop || exit 1
xset s off s noblank -dpms 2>/dev/null || true

cua-driver serve \
    --permission-mode standard \
    --capability-manifest "$MANIFEST" \
    --approve-capability-manifest &
wait_for driver_socket 150 test -S "$HOME/.cache/cua-driver/cua-driver.sock" || exit 1
mark session_ready
log "desktop ready on :0, viewer on 6080"

# The session lives as long as its X server does.
wait
