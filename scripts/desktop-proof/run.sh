#!/usr/bin/env bash
#
# The desktop proof itself. It runs INSIDE the proof container that scripts/desktop-proof.sh
# starts, against binaries bind-mounted from the build tree, and writes everything it sees under
# <work>/results.
#
#   run.sh <repo> <store> <reference> <work> <hold-seconds>
#
# In order: start soma-netd and soma-api; create one desktop sandbox through soma-api; run the
# session launcher through exec; drive the desktop with cua-driver through exec (screenshot, the
# Openbox menu, the terminal); time the agent's click; read idle memory; destroy the sandbox; prove
# nothing was left behind.

set -euo pipefail

REPO="${1:?repo}"
STORE="${2:?store}"
REFERENCE="${3:?reference}"
WORK="${4:?work}"
HOLD="${5:-0}"
LAUNCH_SAMPLES="${SOMA_PROOF_LAUNCH_SAMPLES:-5}"
OUT="$WORK/results"
STATE="$WORK/state"
HEADS="$WORK/heads"
PROOF="$REPO/scripts/desktop-proof"
CALL=/usr/local/bin/soma-desktop-call
CAPTURES=/run/soma-desktop/captures
mkdir -p "$OUT" "$STATE" "$HEADS"
# shellcheck source=scripts/desktop-proof/api.sh
source "$PROOF/api.sh"

log() { printf '\n==> %s\n' "$1" | tee -a "$OUT/calls.log"; }
ns() { date +%s%N; }
record() { printf '%s\t%s\n' "$1" "$2" >>"$OUT/measurements.tsv"; }

start_runtime() {
    local address gateway
    address="$(ip -4 -o addr show dev eth0 | awk '{print $4}' | cut -d/ -f1)"
    gateway="$(ip -4 route show default | awk '{print $3}')"
    install -d -m 0750 -o 0 -g 0 /run/soma-netd  # soma-netd refuses any other mode for its socket directory
    "$REPO/target/release/soma-netd" --uplink eth0 --socket-group 0 --lifecycle-uid 0 \
        --reconcile-uid 0 --connect-ports 6080 --resolver 1.1.1.1 \
        --host-address "$address" --host-address "$gateway" >"$OUT/netd.log" 2>&1 &
    for _ in $(seq 1 100); do [[ -S /run/soma-netd/broker.sock ]] && break; sleep 0.1; done
    SOMA_GENERATION_STORE="$STORE" SOMA_HEAD_DIR="$HEADS" SOMA_PREPARED_MACHINES=0 \
    SOMA_NETD_SOCKET=/run/soma-netd/broker.sock SOMA_NETD_UPLINK=eth0 \
    SOMA_NETD_RESOLVERS=1.1.1.1 SOMA_NETD_HOST_ADDRESSES="$address,$gateway" \
    SOMA_API_TUNNEL_PORTS=6080 \
        "$REPO/target/release/soma-api" --listen "$API" --backend kvm --state-root "$STATE" \
        --workers 2 >"$OUT/api.log" 2>&1 &
    for _ in $(seq 1 300); do
        curl -s -o /dev/null -H "x-soma-tenant: $TENANT" "http://$API/v1/sandboxes" && return 0
        sleep 0.1
    done
    printf 'soma-api did not start; see %s/api.log\n' "$OUT" >&2
    return 1
}

# What a leak would show up as: processes naming the state root, namespaces, nft tables, links,
# files under the heads directory, and the state root's own contents.
probe() {
    local when="$1"
    pgrep -a -f "$STATE" | grep -v -e 'soma-api --listen' -e 'pgrep' | sort >"$OUT/probe.$when.processes" || true
    ip netns list | awk '{print $1}' | sort >"$OUT/probe.$when.netns"
    nft list tables | sort >"$OUT/probe.$when.nft"
    ip -br link | awk '{print $1}' | sort >"$OUT/probe.$when.links"
    find "$HEADS" -type f | sort >"$OUT/probe.$when.heads"
    find "$STATE" | sort >"$OUT/probe.$when.state"
}

machine_memory() {
    local pid best=0 rss
    for pid in $(pgrep -f 'soma-api machine-host'); do
        rss="$(awk '/^VmRSS/{print $2}' "/proc/$pid/status" 2>/dev/null || echo 0)"
        printf '%s %s kB rss, %s kB pss\n' "$pid" "$rss" \
            "$(awk '/^Pss:/{print $2}' "/proc/$pid/smaps_rollup" 2>/dev/null || echo ?)" >>"$OUT/machine-hosts.$1"
        (( rss > best )) && best="$rss"
    done
    printf '%s\n' "$best"
}

desktop_call() { guest "$1" "$CALL" call "${@:2}"; }

log "starting soma-netd and soma-api"
start_runtime
probe before

SHAPE="{\"vcpu_count\":2,\"memory_mib\":4096,\"storage_mib\":20480,\"capabilities\":{\"network\":$RUNTIME_DEFAULT_NETWORK}}"
log "launch to Ready: $LAUNCH_SAMPLES launches of $REFERENCE, each destroyed at once"
for i in $(seq 1 "$LAUNCH_SAMPLES"); do
    api "launch-$i" POST /v1/sandboxes "{\"image\":\"$REFERENCE\",\"shape\":$SHAPE}"
    [[ "$STATUS" == 201 ]] || { cat "$LAST" >&2; exit 1; }
    record "launch_sample_ms" "$(python3 -c "print(round(float('$SECONDS_TAKEN') * 1000, 1))")"
    record "launch_sample_receipt_ready_ms" "$(field 'next(m["elapsed_ns"] for m in d["receipt"]["milestones"] if m["kind"] == "ready") / 1e6')"
    api "launch-$i-destroy" DELETE "/v1/sandboxes/$(field 'd["result"]["instance_id"]')"
done

log "create: $REFERENCE at the small shape"
created_at="$(ns)"
api create POST /v1/sandboxes "{\"image\":\"$REFERENCE\",\"shape\":$SHAPE}"
ready_at="$(ns)"
[[ "$STATUS" == 201 ]] || { cat "$LAST" >&2; exit 1; }
INSTANCE="$(field 'd["result"]["instance_id"]')"
record launch_to_ready_ms "$(( (ready_at - created_at) / 1000000 ))"

log "session launcher through exec"
guest session-start /usr/local/sbin/soma-desktop-session start
launcher_done="$(ns)"
cp "$GUEST_OUT" "$OUT/session-start.json"
record launcher_exec_ms "$(( (launcher_done - ready_at) / 1000000 ))"

log "cua-driver doctor and the driver's own environment"
guest doctor "$CALL" doctor
cp "$GUEST_OUT" "$OUT/doctor.txt"
# The single quotes are deliberate: the substitution runs in the guest's shell, not this one.
# shellcheck disable=SC2016
guest driver-environment /bin/sh -c \
    'tr "\0" "\n" < /proc/$(pgrep -f "cua-driver serve" | head -1)/environ | grep "^CUA_" | sort; cat /home/ubuntu/.cua-driver/config.json'
cp "$GUEST_OUT" "$OUT/driver-environment.txt"

log "agent: screenshot, a click that opens the Openbox menu, the terminal"
desktop_call desktop-state get_desktop_state "{\"max_image_dimension\":0,\"screenshot_out_file\":\"$CAPTURES/desktop.png\"}"
fetch "$CAPTURES/desktop.png" "$OUT/desktop.png"
record ready_to_first_screenshot_ms "$(( ($(ns) - ready_at) / 1000000 ))"
desktop_call menu-click click '{"x":960,"y":540,"button":"right","target":{"kind":"desktop","display_id":"primary"}}'
desktop_call menu-state get_desktop_state "{\"max_image_dimension\":0,\"screenshot_out_file\":\"$CAPTURES/menu.png\"}"
fetch "$CAPTURES/menu.png" "$OUT/menu.png"
desktop_call menu-close press_key '{"key":"Escape","delivery_mode":"foreground","target":{"kind":"desktop","display_id":"primary"}}'
desktop_call terminal-launch launch_app '{"launch_path":"/usr/bin/xfce4-terminal"}'
TERMINAL_PID="$(field 'd["pid"]' "$GUEST_OUT")"
TERMINAL_WINDOW=""
for _ in 1 2 3 4 5 6 7 8 9 10; do
    desktop_call terminal-windows list_windows '{}'
    TERMINAL_WINDOW="$(field "next((w['window_id'] for w in d['windows'] if w['pid'] == $TERMINAL_PID), '')" "$GUEST_OUT")"
    [[ -n "$TERMINAL_WINDOW" ]] && break
    sleep 0.5
done
[[ -n "$TERMINAL_WINDOW" ]] || { printf 'the terminal never mapped a window\n' >&2; exit 1; }
desktop_call terminal-type type_text "{\"pid\":$TERMINAL_PID,\"window_id\":$TERMINAL_WINDOW,\"text\":\"mkdir -p /tmp/soma-m1-typed && cd /tmp/soma-m1-typed\\n\"}"
desktop_call terminal-verify verify_state "{\"pid\":$TERMINAL_PID,\"window_id\":$TERMINAL_WINDOW,\"timeout_ms\":5000,\"expect\":[{\"element\":{\"selector\":{\"role\":\"frame\",\"label_contains\":\"soma-m1-typed\"},\"exists\":true}}]}"
desktop_call terminal-tree get_window_state "{\"pid\":$TERMINAL_PID,\"window_id\":$TERMINAL_WINDOW,\"include_screenshot\":false}"
desktop_call terminal-state get_desktop_state "{\"max_image_dimension\":0,\"screenshot_out_file\":\"$CAPTURES/terminal.png\"}"
fetch "$CAPTURES/terminal.png" "$OUT/terminal.png"

log "agent click: five clicks, each one exec request, sent exactly as any other caller would send it"
for i in 1 2 3 4 5; do
    clicked_at="$(ns)"
    desktop_call "agent-click-$i" click '{"x":1400,"y":300,"target":{"kind":"desktop","display_id":"primary"}}'
    record agent_click_ms "$(( ($(ns) - clicked_at) / 1000000 ))"
done

log "idle memory after 30 s, and every connection the guest holds"
sleep 30
# Telemetry and update checks would show here as a remote endpoint that is not loopback; with no
# viewer, the guest is expected to hold no remote connection at all.
guest connections /bin/sh -c 'cat /proc/net/tcp /proc/net/udp'
python3 "$PROOF/connections.py" "$GUEST_OUT" | tee "$OUT/guest-connections.txt"
guest meminfo /bin/cat /proc/meminfo
cp "$GUEST_OUT" "$OUT/guest-meminfo.txt"
record machine_host_rss_kib "$(machine_memory idle)"

if (( HOLD > 0 )); then
    log "holding the desktop for $HOLD s (or until $WORK/release exists)"
    for _ in $(seq 1 "$HOLD"); do [[ -e "$WORK/release" ]] && break; sleep 1; done
fi

log "destroy"
destroy_at="$(ns)"
api destroy DELETE "/v1/sandboxes/$INSTANCE"
record destroy_ms "$(( ($(ns) - destroy_at) / 1000000 ))"
sleep 2
probe after
bash "$PROOF/residue.sh" "$OUT" | tee "$OUT/residue.txt"
printf 'instance\t%s\n' "$INSTANCE" >>"$OUT/measurements.tsv"
cat "$OUT/measurements.tsv"
