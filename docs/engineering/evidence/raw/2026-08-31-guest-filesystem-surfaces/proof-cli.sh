#!/usr/bin/env bash
#
# The command-line proof: the same six operations, through `soma machine file`, retaining the
# exact command and the exact JSON envelope each one produced.
set -euo pipefail

REPO=/srv/soma/guestfs/repo
OUT=${1:?output directory}
STORE=${2:?prepared store directory}

mkdir -p "$OUT"
cd "$REPO"
source ~/.cargo/env

export SOMA_GENERATION_STORE="$STORE"
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1
export SOMA_HEAD_DIR=/srv/soma/guestfs/heads
export SOMA_PREPARED_MACHINES=0
mkdir -p "$SOMA_HEAD_DIR"
STATE=/srv/soma/guestfs/cli-state
rm -rf "$STATE"; mkdir -p "$STATE"
SOMA="./target/release/soma --format json --backend kvm --state-root $STATE"

STEP=0
run() {
    local name="$1"; shift
    STEP=$((STEP + 1))
    local stem
    stem="$(printf '%02d-%s' "$STEP" "$name")"
    printf 'soma %s\n' "$*" > "$OUT/$stem.command"
    set +e
    $SOMA "$@" > "$OUT/$stem.stdout" 2> "$OUT/$stem.stderr"
    local code=$?
    set -e
    printf '%s\n' "$code" > "$OUT/$stem.exit"
    printf '%s -> exit %s\n' "$name" "$code"
}

python3 - "$OUT/binary.bin" <<'PY'
import sys
open(sys.argv[1], 'wb').write(bytes([0x00, 0xff, 0xfe, 0x80, 0x0a, 0x7f, 0xc3, 0x28, 0x00, 0xed, 0xa0, 0x80]))
PY
printf 'hello from the command line' > "$OUT/hello.txt"

run launch machine launch node:22
INSTANCE=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["result"]["instance_id"])' "$OUT/01-launch.stdout")
printf '%s\n' "$INSTANCE" > "$OUT/instance-id"
echo "instance: $INSTANCE"

run mkdir  machine file mkdir  --instance-id "$INSTANCE" /workspace/cliproof
run write  machine file write  --instance-id "$INSTANCE" --content-file "$OUT/hello.txt" /workspace/cliproof/hello.txt
run read   machine file read   --instance-id "$INSTANCE" /workspace/cliproof/hello.txt
run list   machine file list   --instance-id "$INSTANCE" /workspace/cliproof
run exists machine file exists --instance-id "$INSTANCE" /workspace/cliproof/hello.txt
run remove machine file remove --instance-id "$INSTANCE" /workspace/cliproof/hello.txt
run exists-after machine file exists --instance-id "$INSTANCE" /workspace/cliproof/hello.txt

run write-binary machine file write --instance-id "$INSTANCE" --content-file "$OUT/binary.bin" /workspace/cliproof/blob.bin
run read-binary  machine file read  --instance-id "$INSTANCE" /workspace/cliproof/blob.bin
python3 - "$OUT/10-read-binary.stdout" "$OUT/binary-returned.bin" <<'PY'
import base64, json, sys
document = json.load(open(sys.argv[1]))
open(sys.argv[2], 'wb').write(base64.b64decode(document["result"]["content"]["data"]))
PY
# The human format writes the file's bytes and nothing else, so a redirect reproduces it exactly.
$SOMA --format human machine file read --instance-id "$INSTANCE" /workspace/cliproof/blob.bin \
    > "$OUT/binary-human.bin" 2>"$OUT/binary-human.stderr" || true
{
    if cmp -s "$OUT/binary.bin" "$OUT/binary-returned.bin"; then
        echo "BINARY ROUND TRIP (json envelope): identical"
    else
        echo "BINARY ROUND TRIP (json envelope): DIFFERENT"
    fi
    if cmp -s "$OUT/binary.bin" "$OUT/binary-human.bin"; then
        echo "BINARY ROUND TRIP (human redirect): identical"
    else
        echo "BINARY ROUND TRIP (human redirect): DIFFERENT"
    fi
    sha256sum "$OUT/binary.bin" "$OUT/binary-returned.bin" "$OUT/binary-human.bin"
} | tee "$OUT/binary-verdict"

run fail-outside-tree machine file read  --instance-id "$INSTANCE" relative/escape
run fail-absent       machine file read  --instance-id "$INSTANCE" /workspace/cliproof/never-written
# procfs is mounted read-only in the guest, so this is a write the guest itself refuses.
run fail-readonly     machine file write --instance-id "$INSTANCE" --content-file "$OUT/hello.txt" /proc/version
run fail-wrong-kind   machine file read  --instance-id "$INSTANCE" /workspace/cliproof
run fail-not-empty    machine file remove --instance-id "$INSTANCE" /workspace/cliproof

run destroy machine destroy --instance-id "$INSTANCE"
echo "done"
