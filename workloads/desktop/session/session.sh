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

# Xvfb exposes its framebuffer in shared memory for the on-demand viewer.
# SOMA_DESKTOP_SCREEN can also select the diagnostic KasmVNC or dummy Xorg screen.
screen=${SOMA_DESKTOP_SCREEN:-xvfb}
if [[ "$screen" == xvfb ]] && command -v Xvfb >/dev/null; then
    Xvfb :0 -screen 0 "${SOMA_DESKTOP_GEOMETRY:-1920x1080}x24" -shmem -ac -nolisten tcp -noreset \
        +extension RANDR +extension RENDER +extension XTEST >"$STATE/logs/xvfb.log" 2>&1 &
elif [[ "$screen" == kasmvnc ]] && command -v Xkasmvnc >/dev/null; then
    Xkasmvnc :0 \
        -publicIP 127.0.0.1 \
        -httpd /usr/share/kasmvnc/www \
        -geometry "${SOMA_DESKTOP_GEOMETRY:-1920x1080}" \
        -depth 24 \
        -websocketPort 6082 \
        -SecurityTypes None \
        -disableBasicAuth \
        -interface 0.0.0.0 \
        -AlwaysShared \
        -AcceptSetDesktopSize=1 \
        -AcceptKeyEvents \
        -AcceptPointerEvents \
        -SendCutText \
        -AcceptCutText \
        -FrameRate=30 \
        -nolisten tcp \
        >"$STATE/logs/kasmvnc.log" 2>&1 &
else
    # Xorg with the dummy driver: a frame buffer in memory, no device, no VNC. It runs as the
    # desktop user, started directly (not through the setuid wrapper), and opens no terminal.
    /usr/lib/xorg/Xorg :0 \
        -config /etc/soma-desktop/xorg.conf \
        -logfile "$STATE/logs/Xorg.log" \
        -nolisten tcp \
        -noreset \
        -novtswitch \
        -sharevts \
        -keeptty \
        +extension RANDR +extension RENDER +extension XTEST &
fi
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
# The MIOSA look: a static wallpaper, the top bar and the dock. Each is cosmetic, so
# a part that fails to start leaves a working desktop rather than no desktop.
# The desktop itself is PCManFM's: it draws the wallpaper (again on every resize, as the viewer
# follows the browser window), shows ~/Desktop as icons with files, folders and launchers, takes
# drag-to-select and the right-click menu, and its Desktop Settings let a person pick or add any
# wallpaper. If it cannot start, hsetroot paints the MIOSA wallpaper instead (-root: the screen
# server has no XINERAMA, so hsetroot would otherwise find no monitor to draw on).
# Kept running: if it ever stops, the picture it left on the root window is gone but still named
# there, and a new one would draw from it and stop too, so the names are cleared first.
(
    while :; do
        xprop -root -remove _XROOTPMAP_ID 2>/dev/null
        xprop -root -remove _XROOTMAP_ID 2>/dev/null
        xprop -root -remove ESETROOT_PMAP_ID 2>/dev/null
        pcmanfm --profile miosa --desktop >>"$STATE/logs/desktop.log" 2>&1
        sleep 1
    done
) &
sleep 1
pgrep -u "$(id -u)" -x pcmanfm >/dev/null \
    || hsetroot -root -cover /usr/share/backgrounds/miosa/miosa-aurora.jpg >/dev/null 2>&1 \
    || xsetroot -solid '#0a0b14' 2>/dev/null || true
picom --config /etc/soma-desktop/picom.conf >"$STATE/logs/compositor.log" 2>&1 &
xset s off s noblank -dpms 2>/dev/null || true
tint2 -c /etc/soma-desktop/tint2/bar.tint2rc >"$STATE/logs/bar.log" 2>&1 &
# The dock is Plank: frosted glass, icons that grow under the pointer, open windows marked. Its
# settings and the MIOSA theme come from the image (dconf "local" and /usr/share/plank/themes).
plank >"$STATE/logs/dock.log" 2>&1 &

cua-driver serve \
    --permission-mode standard \
    --capability-manifest "$MANIFEST" \
    --approve-capability-manifest &
wait_for driver_socket 150 test -S "$HOME/.cache/cua-driver/cua-driver.sock" || exit 1

# The MIOSA image layer includes desktopd. The platform provisions its token before starting
# the session. desktopd reads it before binding, so no token means no desktop API.
if [[ -x /usr/local/bin/desktopd ]]; then
    ENVD_DESKTOP_BACKEND=cua /usr/local/bin/desktopd >"$STATE/logs/desktopd.log" 2>&1 &
    desktopd_pid=$!
    desktopd_ready() {
        kill -0 "$desktopd_pid" 2>/dev/null && port_open 49985
    }
    wait_for desktop_api 50 desktopd_ready || exit 1
fi
mark session_ready
log "desktop ready on :0"

# The session lives as long as its X server does.
wait
