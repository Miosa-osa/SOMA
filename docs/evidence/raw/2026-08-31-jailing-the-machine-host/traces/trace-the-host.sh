#!/bin/bash
# Traces every syscall the durable machine host issues across a full lifecycle.
# The host is spawned by `machine launch`; strace -ff follows it and stays attached
# for the host's whole life, so the trace spans restore, exec, inspect and release.
set -u
BIN=/srv/soma/jh-target/release/soma
export SOMA_GENERATION_STORE=/srv/soma/dm-store
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1
OUT="$1"; mkdir -p "$OUT/traces"
export SOMA_HEAD_DIR="$OUT/heads"; mkdir -p "$SOMA_HEAD_DIR"
STATE="$OUT/state"; mkdir -p "$STATE"
ID=$(cat /proc/sys/kernel/random/uuid | tr -d -)
echo "instance=$ID"
S="--format json --backend kvm --state-root $STATE"
strace -ff -qq -o "$OUT/traces/t" -e trace=all \
  $BIN $S machine launch --instance-id "$ID" --vcpus 1 --memory-mib 1024 --storage-mib 2048 busybox:stable-musl \
  > "$OUT/1-launch.json" 2>"$OUT/1-launch.err" &
TRACER=$!
# The launch returns as soon as the host answers; wait for the socket to appear.
for i in $(seq 1 600); do [ -S "$STATE/machines/$ID.sock" ] && break; sleep 0.2; done
sleep 2
$BIN $S machine exec --instance-id "$ID" -- /bin/sh -c 'echo persisted-by-the-first-process > /tmp/proof.txt' >"$OUT/2.json" 2>&1; echo "exec-w=$?"
$BIN $S machine exec --instance-id "$ID" -- /bin/cat /tmp/proof.txt >"$OUT/3.json" 2>&1; echo "exec-r=$?"
$BIN $S machine inspect --instance-id "$ID" >"$OUT/4.json" 2>&1; echo "inspect=$?"
$BIN $S machine destroy --instance-id "$ID" >"$OUT/5.json" 2>&1; echo "destroy=$?"
wait $TRACER 2>/dev/null
echo "traces: $(ls -1 "$OUT/traces" | wc -l) files"
