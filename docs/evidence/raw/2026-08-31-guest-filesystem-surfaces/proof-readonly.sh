#!/usr/bin/env bash
#
# Which locations in the guest are genuinely read-only, and what a write into each answers.
#
# The first attempt used /proc/version and got the catch-all `failed` rather than `denied`, so
# this asks the guest what it actually mounts read-only and writes into those, recording the
# typed cause each one produces.
set -euo pipefail

REPO=/srv/soma/guestfs/repo
OUT=${1:?output directory}
STORE=${2:?prepared store directory}
PORT=${3:-18902}

mkdir -p "$OUT"
cd "$REPO"
source ~/.cargo/env

export SOMA_GENERATION_STORE="$STORE"
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1
export SOMA_HEAD_DIR=/srv/soma/guestfs/heads
export SOMA_PREPARED_MACHINES=0
mkdir -p "$SOMA_HEAD_DIR"
STATE=/srv/soma/guestfs/ro-state
rm -rf "$STATE"; mkdir -p "$STATE"

./target/release/soma-api --listen "127.0.0.1:$PORT" --backend kvm --state-root "$STATE" \
    > "$OUT/server.log" 2>&1 &
SERVER=$!
trap 'kill "$SERVER" 2>/dev/null || true' EXIT
for _ in $(seq 1 60); do
    curl -s -o /dev/null "http://127.0.0.1:$PORT/v1/sandboxes" 2>/dev/null && break
    sleep 0.5
done

post() {
    curl -s -X POST "http://127.0.0.1:$PORT$1" -H 'x-soma-tenant: acme' \
        -H 'content-type: application/json' --data-binary "$2"
}

post /v1/sandboxes '{"image":"node:22"}' > "$OUT/00-create.response"
INSTANCE=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["result"]["instance_id"])' "$OUT/00-create.response")
BASE="/v1/sandboxes/$INSTANCE/filesystem"
echo "instance: $INSTANCE"

# What the guest itself says is mounted read-only.
post "/v1/sandboxes/$INSTANCE/commands" \
    '{"executable":"/bin/cat","arguments":["/proc/mounts"]}' > "$OUT/01-mounts.response"
python3 - "$OUT/01-mounts.response" "$OUT/mounts.txt" <<'PY'
import base64, json, sys
document = json.load(open(sys.argv[1]))
open(sys.argv[2], 'wb').write(base64.b64decode(document["result"]["stdout"]["data"]))
PY
echo "--- read-only mounts the guest reports ---"
grep -E '\bro\b|[, ]ro[, ]' "$OUT/mounts.txt" || true

CONTENT=$(printf 'x' | base64 -w0)
step=0
try_write() {
    step=$((step + 1))
    local stem
    stem="$(printf '%02d-write%s' "$step" "$(printf '%s' "$1" | tr '/' '-')")"
    printf '%s\n' "$1" > "$OUT/$stem.path"
    post "$BASE/write" "{\"path\":\"$1\",\"content\":\"$CONTENT\"}" > "$OUT/$stem.response"
    printf '%-34s %s\n' "$1" "$(python3 -c '
import json,sys
d=json.load(open(sys.argv[1]))
r=d.get("result") or {}
print(r.get("refusal") or ("wrote %s bytes" % r.get("byte_length")) if d["status"]=="ok" else "HTTP error %s" % d["error"]["code"])
' "$OUT/$stem.response")"
}

for path in /proc/version /sys/kernel/vmcoreinfo /proc/sys/kernel/hostname /etc/hostname /workspace/plain.txt; do
    try_write "$path"
done

curl -s -X DELETE "http://127.0.0.1:$PORT/v1/sandboxes/$INSTANCE" -H 'x-soma-tenant: acme' \
    > "$OUT/99-destroy.response"
echo done
