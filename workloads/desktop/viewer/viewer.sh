#!/bin/bash
#
# The desktop viewer, run as the desktop user by `soma-desktop-session viewer start`: Sunshine
# captures and encodes the screen, moonlight-web serves the browser client on the viewer port and
# relays the stream to it over WebSockets (the transport that fits the SOMA tunnel). Both were
# paired with each other when the image was built (pair.py), and moonlight-web's default user
# means the page opens without a login: who may open it is the platform's decision, at the tunnel.
#
# It runs only while someone watches, so an unwatched desktop pays nothing for encoding.

set -u

STATE="${SOMA_DESKTOP_STATE:?SOMA_DESKTOP_STATE is set by soma-desktop-session}"
PORT="${SOMA_DESKTOP_VIEWER_PORT:-6080}"
export DISPLAY=:0 XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:?}"
export DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus"

mark() { printf '%s %s\n' "$1" "$(date +%s%N)" >>"$STATE/milestones"; }
port_open() { (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null; }
wait_port() {
    local name="$1" port="$2" i
    for ((i = 0; i < 150; i++)); do
        port_open "$port" && { mark "$name"; return 0; }
        sleep 0.1
    done
    mark "failed_$name"
    return 1
}

stop_children() {
    # Signal the children without signaling this supervisor. Reap both before exiting.
    local pid attempt
    trap '' TERM INT
    while read -r pid; do
        [[ -z "$pid" ]] || kill -TERM "$pid" 2>/dev/null || true
    done <<< "$(jobs -p)"
    for ((attempt = 0; attempt < 30; attempt++)); do
        [[ -n "$(jobs -p)" ]] || break
        sleep 0.1
    done
    while read -r pid; do
        [[ -z "$pid" ]] || kill -KILL "$pid" 2>/dev/null || true
    done <<< "$(jobs -p)"
    wait
}

trap stop_children EXIT
trap 'exit 0' TERM INT

sunshine "$HOME/.config/sunshine/sunshine.conf" >"$STATE/logs/sunshine.log" 2>&1 &
wait_port viewer_encoder 47989 || exit 1

cd /opt/moonlight-web
./web-server --config-path "$HOME/.config/moonlight-web/config.json" --bind-address "0.0.0.0:$PORT" run \
    >"$STATE/logs/moonlight-web.log" 2>&1 &
wait_port viewer_port "$PORT" || exit 1
mark viewer_ready

wait -n
