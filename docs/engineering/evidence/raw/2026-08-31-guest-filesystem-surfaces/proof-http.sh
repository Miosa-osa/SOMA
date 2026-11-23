#!/usr/bin/env bash
#
# The HTTP proof: create a sandbox, then drive all six filesystem operations against it,
# retaining every request and response envelope exactly as they crossed the wire.
set -euo pipefail

REPO=/srv/soma/guestfs/repo
OUT=${1:?output directory}
STORE=${2:?prepared store directory}
PORT=${3:-18901}

mkdir -p "$OUT"
cd "$REPO"
source ~/.cargo/env

export SOMA_GENERATION_STORE="$STORE"
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1
export SOMA_HEAD_DIR=/srv/soma/guestfs/heads
export SOMA_PREPARED_MACHINES=0
mkdir -p "$SOMA_HEAD_DIR"
STATE=/srv/soma/guestfs/api-state
rm -rf "$STATE"; mkdir -p "$STATE"

./target/release/soma-api --listen "127.0.0.1:$PORT" --backend kvm --state-root "$STATE" \
    > "$OUT/server.log" 2>&1 &
SERVER=$!
trap 'kill "$SERVER" 2>/dev/null || true' EXIT
for _ in $(seq 1 60); do
    if curl -s -o /dev/null "http://127.0.0.1:$PORT/v1/sandboxes" 2>/dev/null; then break; fi
    sleep 0.5
done

# One numbered record per exchange: the exact request, and the exact response.
STEP=0
call() {
    local name="$1" method="$2" path="$3" body="${4-}"
    STEP=$((STEP + 1))
    local stem
    stem="$(printf '%02d-%s' "$STEP" "$name")"
    {
        printf '%s http://127.0.0.1:%s%s\n' "$method" "$PORT" "$path"
        printf 'x-soma-tenant: acme\n'
        printf 'content-type: application/json\n\n'
        printf '%s\n' "${body:-<no body>}"
    } > "$OUT/$stem.request"
    local -a data=()
    [[ -n "$body" ]] && data=(--data-binary "$body")
    local status
    status=$(curl -s -o "$OUT/$stem.response" -w '%{http_code}' \
        -X "$method" "http://127.0.0.1:$PORT$path" \
        -H 'x-soma-tenant: acme' -H 'content-type: application/json' \
        "${data[@]}")
    printf '%s\n' "$status" > "$OUT/$stem.status"
    printf '%s %s -> %s\n' "$method" "$path" "$status"
}

# The bytes the round trip has to preserve: not valid UTF-8 anywhere in them.
python3 - "$OUT/binary.bin" <<'PY'
import sys
open(sys.argv[1], 'wb').write(bytes([0x00, 0xff, 0xfe, 0x80, 0x0a, 0x7f, 0xc3, 0x28, 0x00, 0xed, 0xa0, 0x80]))
PY
BINARY_B64=$(base64 -w0 < "$OUT/binary.bin")
HELLO_B64=$(base64 -w0 <<< "hello from the host")

call create POST /v1/sandboxes '{"image":"node:22"}'
INSTANCE=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["result"]["instance_id"])' "$OUT/01-create.response")
printf '%s\n' "$INSTANCE" > "$OUT/instance-id"
echo "instance: $INSTANCE"

BASE="/v1/sandboxes/$INSTANCE/filesystem"
call mkdir  POST "$BASE/mkdir"  '{"path":"/workspace/proof"}'
call write  POST "$BASE/write"  "{\"path\":\"/workspace/proof/hello.txt\",\"content\":\"$HELLO_B64\"}"
call read   POST "$BASE/read"   '{"path":"/workspace/proof/hello.txt"}'
call list   POST "$BASE/list"   '{"path":"/workspace/proof"}'
call exists POST "$BASE/exists" '{"path":"/workspace/proof/hello.txt"}'
call remove POST "$BASE/remove" '{"path":"/workspace/proof/hello.txt"}'
call exists-after POST "$BASE/exists" '{"path":"/workspace/proof/hello.txt"}'

# Binary safety: exactly these bytes back out again.
call write-binary POST "$BASE/write" \
    "{\"path\":\"/workspace/proof/blob.bin\",\"content\":\"$BINARY_B64\"}"
call read-binary  POST "$BASE/read"  '{"path":"/workspace/proof/blob.bin"}'
python3 - "$OUT/10-read-binary.response" "$OUT/binary-returned.bin" <<'PY'
import base64, json, sys
document = json.load(open(sys.argv[1]))
open(sys.argv[2], 'wb').write(base64.b64decode(document["result"]["content"]["data"]))
PY
if cmp -s "$OUT/binary.bin" "$OUT/binary-returned.bin"; then
    echo "BINARY ROUND TRIP: identical" | tee "$OUT/binary-verdict"
else
    echo "BINARY ROUND TRIP: DIFFERENT" | tee "$OUT/binary-verdict"
fi
sha256sum "$OUT/binary.bin" "$OUT/binary-returned.bin" >> "$OUT/binary-verdict"

# The three failure cases, each of which must name the cause that happened.
call fail-outside-tree POST "$BASE/read"  '{"path":"relative/escape"}'
call fail-absent       POST "$BASE/read"  '{"path":"/workspace/proof/never-written"}'
# procfs is mounted read-only in the guest, so this is a write the guest itself refuses rather
# than one the overlay would quietly accept.
call fail-readonly     POST "$BASE/write" "{\"path\":\"/proc/version\",\"content\":\"$HELLO_B64\"}"
call fail-wrong-kind   POST "$BASE/read"  '{"path":"/workspace/proof"}'
call fail-not-empty    POST "$BASE/remove" '{"path":"/workspace/proof"}'

call destroy DELETE "/v1/sandboxes/$INSTANCE" ''
echo "done"
