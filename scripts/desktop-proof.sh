#!/usr/bin/env bash
#
# The desktop proof (milestone M1): one SOMA machine runs Xfce on KasmVNC's X server, cua-driver
# inside it takes a screenshot, clicks, types into a real terminal, and the effect is read back by
# the guest agent, not by the driver.
#
#   scripts/desktop-proof.sh <work-dir>
#
# Run it from the repository root on a Linux KVM host as an ordinary user in the kvm and docker
# groups; nothing in it needs root. The pinned filesystem tools are found through SOMA_FS_TOOLS, or
# through SOMA_EROFS_TOOLS (<fs-tools>/erofs) as the check host exports it. The work directory
# keeps the compiled Generation between runs (an unchanged image is restored, not recompiled) and
# gets one run-<stamp> directory per run with every command, its output, its exit code and its
# duration (out/steps.log), the measurements (out/measurements.tsv) and the ten pass lines
# (out/pass.tsv). The machine has no network device: nothing reaches it over a network in M1.
#
# DESKTOP_PROOF_SKIP_BUILD=1 reuses the SOMA binaries already in target/ (for iterating on a host
# where they were built from this checkout).

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$REPO_ROOT" || exit 2
W="${1:?usage: scripts/desktop-proof.sh <work-dir>}"
FS="${SOMA_FS_TOOLS:-$(dirname "${SOMA_EROFS_TOOLS:?set SOMA_FS_TOOLS or SOMA_EROFS_TOOLS}")}"
NONCE="$(date +%s)"
mkdir -p "$W/store" "$W/run-$NONCE"/{out,state,heads} || exit 2
W="$(cd "$W" && pwd -P)"
STORE="$W/store" RUN="$W/run-$NONCE"
OUT="$RUN/out" STATE="$RUN/state" HEADS="$RUN/heads"
SOMA=(./target/release/soma --backend kvm --state-root "$STATE")
export SOMA_GENERATION_STORE="$STORE" SOMA_HEAD_DIR="$HEADS"
STEP=0 STEP_OUT="" ID=""

ns() { date +%s%N; }
record() { printf '%s\t%s\n' "$1" "$2" | tee -a "$OUT/measurements.tsv"; }
pass() { printf '%s\t%s\t%s\n' "$1" "$2" "$3" | tee -a "$OUT/pass.tsv"; }

# step <name> <command...>: runs one command, keeps its output in out/NN-name.out, logs it, and
# stops the whole run at the first failure, naming the step.
step() {
    local name="$1" started status
    shift
    STEP=$((STEP + 1))
    STEP_OUT="$(printf '%s/%02d-%s.out' "$OUT" "$STEP" "$name")"
    started="$(ns)"
    "$@" >"$STEP_OUT" 2>&1
    status=$?
    STEP_MS=$(( ($(ns) - started) / 1000000 ))
    printf '%02d %-24s exit %-3s %6s ms  %s\n' "$STEP" "$name" "$status" "$STEP_MS" "$*" | tee -a "$OUT/steps.log"
    if (( status != 0 )); then
        tail -30 "$STEP_OUT" | tee -a "$OUT/steps.log"
        printf 'FAILED at step %02d %s\n' "$STEP" "$name" | tee -a "$OUT/steps.log"
        [[ -z "$ID" ]] || "${SOMA[@]}" machine destroy --instance-id "$ID" >/dev/null 2>&1
        exit 1
    fi
}

# json <python expression on d> <file>: one value out of a JSON file.
json() { python3 -c 'import json,sys; d=json.load(open(sys.argv[2])); print(eval(sys.argv[1]))' "$1" "$2"; }

# x <command...>: one command in the guest; its decoded stdout is the step's output.
x() {
    "${SOMA[@]}" --format json machine exec --instance-id "$ID" --timeout-ms 120000 \
        --max-output-bytes 8388608 -- "$@" >"$RUN/exec.json" || return 1
    python3 - "$RUN/exec.json" <<'PY'
import base64, json, sys
result = json.load(open(sys.argv[1]))["result"]
sys.stdout.write(base64.b64decode(result["stdout"]["data"]).decode(errors="replace"))
sys.stderr.write(base64.b64decode(result["stderr"]["data"]).decode(errors="replace"))
sys.exit(result["execution"].get("exited", {}).get("code", 125))
PY
}
c() { x /usr/local/bin/cua-call "$1" "${2:-"{}"}"; }
# structured <python expression on s> <file>: a value out of a cua-call answer's structuredContent.
structured() { json "(lambda s: $1)(d['result'].get('structuredContent', {}))" "$2"; }
read_out() { "${SOMA[@]}" machine file read --instance-id "$ID" "$1" >"$2"; }

printf 'desktop proof %s at %s in %s\n' "$NONCE" "$(git rev-parse --short HEAD)" "$RUN" | tee "$OUT/steps.log"

# 1. SOMA itself: the command line, the static guest agent, the pinned kernel and the two tools.
if [[ "${DESKTOP_PROOF_SKIP_BUILD:-0}" != 1 ]]; then
    step build-soma ./scripts/build-soma.sh
    step build-tools cargo build --release --locked -p soma-generation --example prepare_generation \
        -p soma-local --example capture_snapshot
fi

# 2. The guest image, every download checked against its pinned SHA-256.
step image-build docker build --platform=linux/amd64 -t soma-desktop:m1 workloads/desktop
IMAGE_ID="$(docker image inspect --format '{{.Id}}' soma-desktop:m1)"
# A rebuilt layer prints both checksum lines and the driver's version; a cached one ran them when
# it was built, which the build log says instead.
if (( $(grep -c ': OK$' "$STEP_OUT") == 2 )) && grep -q 'cua-driver 0.55.0' "$STEP_OUT"; then
    pass 1 image-build "exit 0; both sha256 checks OK; cua-driver 0.55.0; $IMAGE_ID"
elif (( $(grep -c 'Using cache' "$STEP_OUT") >= 2 )); then
    pass 1 image-build "exit 0; checked layers taken from the cache they were checked into; $IMAGE_ID"
fi

# 3. The Generation: compiled once per image, then restored. The reference carries the image id,
#    so a changed image gets a new entry and an unchanged one reuses its captured snapshot.
REFERENCE="soma-desktop:m1-${IMAGE_ID:7:12}"
ENTRY="$STORE/ref-$(printf '%s' "$REFERENCE" | sha256sum | cut -d' ' -f1)"
if [[ ! -s "$ENTRY/generation.id" ]]; then
    step image-save docker save -o "$W/image.tar" soma-desktop:m1
    # The image comes from an archive: skopeo 1.13 cannot read the daemon of Docker 29.
    step prepare env SOMA_IMAGE_SOURCE="docker-archive:$W/image.tar" SOMA_PREPARE_VCPUS=2 \
        SOMA_ALLOW_COLD_BOOT_ENTRY=1 ./scripts/prepare-generation.sh "$REFERENCE" "$STORE" "$FS" 4096 20480
    step capture ./target/release/examples/capture_snapshot "$ENTRY" 4096
    rm -f "$W/image.tar"
    boot=COMPILED
else
    boot=RESTORE
fi
[[ -f "$ENTRY/snapshot/state.somasnap" ]] &&
    pass 2 generation "$boot; generation $(cat "$ENTRY/generation.id"); $(du -sh --apparent-size "$ENTRY" | cut -f1) apparent"

# 4. Launch to Ready, five times with an immediate destroy, then the machine the proof uses.
LAUNCH=("${SOMA[@]}" --format json machine launch --vcpus 2 --memory-mib 4096 --storage-mib 20480 "$REFERENCE")
for sample in 1 2 3 4 5; do
    step "launch-sample-$sample" "${LAUNCH[@]}"
    record launch_sample_ms "$STEP_MS"
    record launch_sample_receipt_ready_ms \
        "$(json 'next(m["elapsed_ns"] for m in d["receipt"]["milestones"] if m["kind"] == "ready") / 1e6' "$STEP_OUT")"
    step "destroy-sample-$sample" "${SOMA[@]}" machine destroy --instance-id "$(json 'd["result"]["instance_id"]' "$STEP_OUT")"
done
step launch "${LAUNCH[@]}"
ID="$(json 'd["result"]["instance_id"]' "$STEP_OUT")"
record launch_ms "$STEP_MS"
[[ "$(json 'd["status"]' "$STEP_OUT")" == ok && "$(json 'd["result"]["state"]' "$STEP_OUT")" == ready ]] &&
    pass 3 launch "ok, ready, $STEP_MS ms on the host clock, receipt ready at $(json 'next(m["elapsed_ns"] for m in d["receipt"]["milestones"] if m["kind"] == "ready") / 1e6' "$STEP_OUT") ms"

# 5. The session, with a viewer of port 6080 started beside it in the same command, so the first
#    frame is timed inside the guest against the launcher's own start.
step probe-copy "${SOMA[@]}" machine file write --instance-id "$ID" \
    --content-file scripts/desktop-proof/first_frame.py /tmp/first_frame.py
# shellcheck disable=SC2016  # expanded by the guest's shell, not this one
step session x /bin/sh -c 'python3 /tmp/first_frame.py > /tmp/first-frame.json & /usr/local/sbin/desktop-session; status=$?; wait; exit $status'
cp "$STEP_OUT" "$OUT/session.json"
record session_exec_ms "$STEP_MS"
record session_ms "$(json 'd["session_ms"]' "$OUT/session.json")"
step first-frame x /bin/cat /tmp/first-frame.json
cp "$STEP_OUT" "$OUT/first-frame.json"
step launcher-start x /bin/cat /run/desktop/start
record launcher_start_to_first_frame_ms \
    "$(( ($(json 'd["first_painted_ns"]' "$OUT/first-frame.json") - $(cat "$STEP_OUT")) / 1000000 ))"
step rfb-banner x /bin/sh -c "curl -s -m 3 -i -N -H 'Connection: Upgrade' -H 'Upgrade: websocket' -H 'Origin: http://localhost' -H 'Sec-WebSocket-Version: 13' -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' -H 'Sec-WebSocket-Protocol: binary' http://127.0.0.1:6080/websockify | grep -aq 'RFB 003' && echo desktop_ready"
grep -q desktop_ready "$STEP_OUT" && pass 4 session "exit 0, session_ms $(json 'd["session_ms"]' "$OUT/session.json"), desktop_ready"

# 6. The driver sees the display and the accessibility bus.
step doctor x /usr/local/bin/cua doctor
grep -q 'X11 connection: connected' "$STEP_OUT" && grep -q 'org.a11y.Bus reachable' "$STEP_OUT" &&
    pass 5 driver-health "doctor exit 0; X11 connected; org.a11y.Bus reachable"
step describe-click x /usr/local/bin/cua describe click
step screen-size c get_screen_size
size="$(structured '"%sx%s" % (s.get("width"), s.get("height"))' "$STEP_OUT")"

# 7. A screenshot at native size, read back out of the machine by the guest agent.
step screenshot-before c get_desktop_state '{"max_image_dimension":0,"screenshot_out_file":"/tmp/m1-before.png"}'
step read-before read_out /tmp/m1-before.png "$OUT/m1-before.png"
png="$(file -b "$OUT/m1-before.png")"
[[ "$size" == 1920x1080 && "$png" == "PNG image data, 1920 x 1080"* ]] && pass 6 screen "get_screen_size $size; $png"

# 8. A terminal: open it, click into it, type a command, press Enter, all desktop-wide input.
step terminal-launch c launch_app '{"name":"xfce4-terminal"}'
PID="$(structured 's["pid"]' "$STEP_OUT")"
for _ in 1 2 3 4 5 6 7 8 9 10; do
    step windows c list_windows
    WID="$(structured "next((w['window_id'] for w in s['windows'] if w['pid'] == $PID), '')" "$STEP_OUT")"
    [[ -n "$WID" ]] && break
    sleep 0.5
done
read -r CX CY < <(structured "next(('%d %d' % (w['bounds']['x'] + w['bounds']['width'] // 2, w['bounds']['y'] + w['bounds']['height'] // 2)) for w in s['windows'] if w['window_id'] == $WID)" "$STEP_OUT")
step terminal-click c click "{\"scope\":\"desktop\",\"x\":$CX,\"y\":$CY}"
record click_exec_ms "$STEP_MS"
step terminal-type c type_text "{\"scope\":\"desktop\",\"text\":\"echo cua-m1-$NONCE > /tmp/cua-m1.txt\"}"
record type_text_exec_ms "$STEP_MS"
step terminal-enter c press_key '{"scope":"desktop","key":"enter"}'

# 9. The independent proof: the guest agent, not the driver, reads what the typed command wrote.
for _ in 1 2 3 4 5; do
    if x /bin/cat /tmp/cua-m1.txt >"$RUN/typed.txt" 2>/dev/null && [[ "$(cat "$RUN/typed.txt")" == "cua-m1-$NONCE" ]]; then break; fi
    sleep 1
done
step typed-file x /bin/cat /tmp/cua-m1.txt
[[ "$(cat "$STEP_OUT")" == "cua-m1-$NONCE" ]] && pass 7 input "/tmp/cua-m1.txt holds cua-m1-$NONCE"

# 10. The terminal's accessibility tree, and a second screenshot that must differ from the first.
step terminal-state c get_window_state "{\"pid\":$PID,\"window_id\":$WID,\"include_screenshot\":false}"
cp "$STEP_OUT" "$OUT/terminal-state.json"
elements="$(structured 's.get("element_count", 0)' "$STEP_OUT")"
! grep -q '"degraded":true' "$STEP_OUT" && (( elements > 0 )) && pass 8 accessibility "$elements elements, not degraded"
step screenshot-after c get_desktop_state '{"max_image_dimension":0,"screenshot_out_file":"/tmp/m1-after.png"}'
record get_desktop_state_exec_ms "$STEP_MS"
step read-after read_out /tmp/m1-after.png "$OUT/m1-after.png"
before="$(sha256sum <"$OUT/m1-before.png" | cut -c1-64)" after="$(sha256sum <"$OUT/m1-after.png" | cut -c1-64)"
[[ "$before" != "$after" ]] && pass 9 pixels-changed "before ${before:0:16}, after ${after:0:16}"

# 11. Twenty rounds of a keypress then a screenshot, timed inside the guest around each driver
#     call (the in-guest stand-in for click-to-paint; no exec or sweep in the numbers), and memory.
step rounds x /usr/bin/python3 -c '
import json, subprocess, time
for r in range(20):
    t0 = time.time_ns(); subprocess.run(["/usr/local/bin/cua-call", "press_key", "{\"scope\":\"desktop\",\"key\":\"a\"}"], capture_output=True, check=True)
    t1 = time.time_ns(); subprocess.run(["/usr/local/bin/cua-call", "get_desktop_state", "{\"max_image_dimension\":0,\"screenshot_out_file\":\"/tmp/r.png\"}"], capture_output=True, check=True)
    t2 = time.time_ns(); print(json.dumps({"round": r + 1, "press_key_ms": (t1 - t0) / 1e6, "get_desktop_state_ms": (t2 - t1) / 1e6, "key_to_screenshot_ms": (t2 - t0) / 1e6}))'
cp "$STEP_OUT" "$OUT/rounds.jsonl"
step memory x /usr/bin/free -m

# 12. Destroy, and prove nothing is left: no process names the state root, no file in the heads.
step destroy "${SOMA[@]}" machine destroy --instance-id "$ID"
ID=""
sleep 1
left="$(pgrep -f "$STATE" | wc -l)" files="$(find "$HEADS" -type f | wc -l)"
(( left == 0 && files == 0 )) && pass 10 cleanup "destroy exit 0; 0 processes name the state root; 0 files under the heads"

printf '\n%s of 10 pass lines\n' "$(wc -l <"$OUT/pass.tsv" 2>/dev/null || echo 0)" | tee -a "$OUT/steps.log"
[[ "$(wc -l <"$OUT/pass.tsv")" == 10 ]]
