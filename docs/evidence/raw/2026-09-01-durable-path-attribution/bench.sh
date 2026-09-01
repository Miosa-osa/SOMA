#!/usr/bin/env bash
# One arm of the durable-path measurement. $1 label, $2 repo, $3 results base.
# The results parent is where the harness puts the durable state root, so it selects the
# storage the state store is measured on; SOMA_HEAD_DIR always stays on the XFS that holds
# the reflinked overlay heads.
set -euo pipefail
LABEL="${1:?usage: bench.sh <label> [repo] [results-base]}"
REPO="${2:-/srv/soma/durable-perf/SOMA}"
OUTBASE="${3:-/srv/soma/dp}"
BASE=/srv/soma/durable-perf
OUT="$OUTBASE/$LABEL"
STORE="$BASE/root/store/busybox-stable-musl-1-1024-2048"
source "$HOME/.cargo/env"
mkdir -p "$OUT"
cd "$REPO"
export SOMA_GENERATION_STORE="$STORE"
export SOMA_HEAD_DIR="$BASE/heads-$LABEL"
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1
mkdir -p "$SOMA_HEAD_DIR"

busy() {
    python3 - <<'PY'
import time
def snap():
    v=[int(x) for x in open('/proc/stat').readline().split()[1:]]
    return sum(v), v[3]+v[4]
a=snap(); time.sleep(5); b=snap()
print(f"busy_fraction={1-(b[1]-a[1])/(b[0]-a[0]):.4f} loadavg={open('/proc/loadavg').read().split()[0]}")
PY
}

[[ -f "$OUT/manifest.json" ]] || python3 -m benchmarks.local_alpha.build_release --build-manifest "$OUT/manifest.json"

for run in "${RUNS:-1 2 3}"; do :; done
for run in ${RUNS:-1 2 3}; do
    echo "== $LABEL run $run" | tee -a "$OUT/host.txt"
    busy | tee -a "$OUT/host.txt"
    rm -f "$OUT/run-$run.jsonl"
    set +e
    python3 -m benchmarks.local_alpha.burst run \
        --experiment-class warm-cache-restore --backend kvm \
        --image busybox:stable-musl --iterations 100 --concurrency 100 \
        --vcpus 1 --memory-mib 1024 --storage-mib 2048 \
        --prepared "the Generation store, the host page cache, and the release build" \
        --build-manifest "$OUT/manifest.json" --soma-bin "$REPO/target/release/soma" \
        --soma-mcp-bin "$REPO/target/release/soma-mcp" \
        --results "$OUT/run-$run.jsonl" -- /bin/echo soma-ok > "$OUT/summary-$run.json" 2> "$OUT/stderr-$run.txt"
    echo "burst exit $?" | tee -a "$OUT/host.txt"
    set -e
    echo "residue heads=$(ls -1 "$SOMA_HEAD_DIR" | wc -l) hosts=$(pgrep -c -f 'machine-host' || true)" | tee -a "$OUT/host.txt"
done
echo done
