#!/bin/bash
# Five separate soma processes over one sandbox.
set -u
BIN=${SOMA_BIN:-/srv/soma/jh-target/release/soma}
export SOMA_GENERATION_STORE=${SOMA_GENERATION_STORE:-/srv/soma/dm-store}
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1
OUT="$1"; mkdir -p "$OUT"
export SOMA_HEAD_DIR="$OUT/heads"; mkdir -p "$SOMA_HEAD_DIR"
STATE="$OUT/state"; mkdir -p "$STATE"
ID=$(cat /proc/sys/kernel/random/uuid | tr -d -)
echo "instance=$ID"
S="--format json --backend kvm --state-root $STATE"
run() { n="$1"; shift; "$@" > "$OUT/$n.json" 2>"$OUT/$n.err"; echo "$n exit=$?"; }
run 1-launch  $BIN $S machine launch --instance-id "$ID" --vcpus 1 --memory-mib 1024 --storage-mib 2048 busybox:stable-musl
run 2-exec-w  $BIN $S machine exec --instance-id "$ID" -- /bin/sh -c 'echo persisted-by-the-first-process > /tmp/proof.txt; ls -l /tmp/proof.txt'
run 3-exec-r  $BIN $S machine exec --instance-id "$ID" -- /bin/cat /tmp/proof.txt
run 4-inspect $BIN $S machine inspect --instance-id "$ID"
run 5-destroy $BIN $S machine destroy --instance-id "$ID"
echo "--- release ---"
echo "heads=$(ls -1 "$SOMA_HEAD_DIR" 2>/dev/null | wc -l) sockets=$(ls -1 "$STATE"/machines/*.sock 2>/dev/null | wc -l) hosts=$(pgrep -fc machine-host 2>/dev/null || echo 0)"
