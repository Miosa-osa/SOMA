#!/bin/bash
set -u
cd /srv/soma/jh
export SOMA_GENERATION_STORE=/srv/soma/dm-store
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1
N="$1"
export SOMA_HEAD_DIR=/srv/soma/jh-run/bench-heads-$N
mkdir -p "$SOMA_HEAD_DIR" /srv/soma/jh-run/results
python3 -m benchmarks.local_alpha.burst run --experiment-class warm-cache-restore --backend kvm \
  --image busybox:stable-musl --iterations 100 --concurrency 100 --vcpus 1 --memory-mib 1024 \
  --storage-mib 2048 --prepared "the Generation store, the host page cache, and the release build" \
  --build-manifest /srv/soma/jh-run/manifest.json \
  --soma-bin /srv/soma/jh/target/release/soma \
  --soma-mcp-bin /srv/soma/jh/target/release/soma-mcp \
  --results /srv/soma/jh-run/results/run-$N.json -- /bin/echo soma-ok
echo "EXIT=$?"
echo "--- release after run $N ---"
echo "heads=$(ls -1 "$SOMA_HEAD_DIR" 2>/dev/null | wc -l) hosts=$(pgrep -f "soma machine-host" 2>/dev/null | wc -l)"
