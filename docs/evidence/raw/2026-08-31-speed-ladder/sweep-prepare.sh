#!/usr/bin/env bash
# Builds one Generation per configuration in the speed ladder.
#
# Memory is not a launch parameter: a restore must map exactly the memory the snapshot was
# captured with, so each memory size is a separate Generation and a separate capture. That is
# what makes this a build step rather than a flag, and it is the whole reason the ladder has to
# be prepared before it can be measured.
set -uo pipefail
REPO=/srv/soma/SOMA
TOOLS=/srv/soma/fs-tools
OUT=/srv/soma/bench/sweep-prepare.log
: > "$OUT"

# name              image                  mem   disk
CONFIGS=(
  "bb-128-1024      busybox:stable-musl    128   1024"
  "bb-512-10240     busybox:stable-musl    512   10240"
  "bb-1024-10240    busybox:stable-musl    1024  10240"
  "node-1024-10240  node:22                1024  10240"
)

for row in "${CONFIGS[@]}"; do
    read -r name image mem disk <<< "$row"
    store="/srv/soma/sweep/$name"
    if [[ -f "$store/.ready" ]]; then echo "$name already prepared" | tee -a "$OUT"; continue; fi
    echo "=== $name : $image ${mem}MiB mem ${disk}MiB disk ===" | tee -a "$OUT"
    rm -rf "$store"; mkdir -p "$store"
    if ! timeout 2400 bash "$REPO/scripts/prepare-generation.sh" "$image" "$store" "$TOOLS" "$mem" "$disk" >> "$OUT" 2>&1; then
        echo "$name PREPARE FAILED" | tee -a "$OUT"; continue
    fi
    entry=$(find "$store" -maxdepth 1 -name 'ref-*' | head -1)
    if [[ -z "$entry" ]]; then echo "$name NO ENTRY" | tee -a "$OUT"; continue; fi
    if ! timeout 1200 "$REPO/target/release/examples/capture_snapshot" "$entry" "$mem" >> "$OUT" 2>&1; then
        echo "$name CAPTURE FAILED" | tee -a "$OUT"; continue
    fi
    touch "$store/.ready"
    echo "$name ready" | tee -a "$OUT"
done
echo "=== sweep prepare complete ===" | tee -a "$OUT"
