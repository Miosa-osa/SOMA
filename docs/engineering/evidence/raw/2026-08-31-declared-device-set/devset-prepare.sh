#!/usr/bin/env bash
# Builds the writable and read-only busybox Generations of the device-set comparison.
#
# Both are the same image at the same memory size, so the only difference between them is the
# device set: one declares 1024 MiB of writable storage and the other declares none.
set -uo pipefail
export PATH=$HOME/.cargo/bin:$PATH
REPO=/srv/soma/SOMA-devset
TOOLS=/srv/soma/fs-tools
OUT=/srv/soma/devset-prepare.log
: > "$OUT"
CONFIGS=(
  "ds-rw-128-1024   busybox:stable-musl  128  1024"
  "ds-ro-128-0      busybox:stable-musl  128  0"
)
for row in "${CONFIGS[@]}"; do
    read -r name image mem disk <<< "$row"
    store="/srv/soma/devset/$name"
    if [[ -f "$store/.ready" ]]; then echo "$name already prepared" | tee -a "$OUT"; continue; fi
    echo "=== $name : $image ${mem}MiB mem ${disk}MiB disk ===" | tee -a "$OUT"
    rm -rf "$store"; mkdir -p "$store"
    if ! timeout 2400 bash "$REPO/scripts/prepare-generation.sh" "$image" "$store" "$TOOLS" "$mem" "$disk" >> "$OUT" 2>&1; then
        echo "$name PREPARE FAILED" | tee -a "$OUT"; continue
    fi
    entry=$(find "$store" -maxdepth 1 -name "ref-*" | head -1)
    if [[ -z "$entry" ]]; then echo "$name NO ENTRY" | tee -a "$OUT"; continue; fi
    if ! timeout 1200 "$REPO/target/release/examples/capture_snapshot" "$entry" "$mem" >> "$OUT" 2>&1; then
        echo "$name CAPTURE FAILED" | tee -a "$OUT"; continue
    fi
    touch "$store/.ready"
    echo "$name ready" | tee -a "$OUT"
done
echo "=== devset prepare complete ===" | tee -a "$OUT"
