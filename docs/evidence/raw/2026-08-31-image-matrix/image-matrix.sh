#!/usr/bin/env bash
# Compiles a matrix of OCI images into Generations and proves each one boots and runs a command.
#
# Everything proved so far used node:22 and busybox. A sandbox product has to accept whatever
# image a user names, so this walks a deliberately awkward set: two glibc distributions, a musl
# one, a security distribution with a very large root, three language runtimes, and an image whose
# entrypoint is a daemon. Each is compiled, captured, restored, and asked to run one command that
# proves the workload is really there.
#
# Failures are the point. Each image records what stage it reached, so a failure names the stage
# rather than disappearing.
set -uo pipefail

STORE=/srv/soma/matrix
OUT=/srv/soma/bench/image-matrix.jsonl
REPO=/srv/soma/SOMA
FS_TOOLS=/srv/soma/fs-tools
mkdir -p "$STORE" "$(dirname "$OUT")"
: > "$OUT"

# image                        probe command                      expected substring
CASES=(
  "ubuntu:24.04|/bin/cat /etc/os-release|Ubuntu 24.04"
  "ubuntu:22.04|/bin/cat /etc/os-release|Ubuntu 22.04"
  "debian:12|/bin/cat /etc/os-release|Debian GNU/Linux 12"
  "alpine:3.20|/bin/cat /etc/os-release|Alpine Linux"
  "python:3.12|/usr/local/bin/python3 --version|Python 3.12"
  "golang:1.23|/usr/local/go/bin/go version|go1.23"
  "node:22|/usr/local/bin/node --version|v22"
  "kalilinux/kali-rolling:latest|/bin/cat /etc/os-release|Kali"
)

record() {
    printf '{"image":"%s","stage":"%s","ok":%s,"detail":"%s"}\n' "$1" "$2" "$3" "$4" >> "$OUT"
}

for case in "${CASES[@]}"; do
    IFS='|' read -r image cmd expect <<< "$case"
    key=$(printf '%s' "$image" | tr '/:' '__')
    echo "=== $image ==="

    if ! timeout 1800 bash "$REPO/scripts/prepare-generation.sh" "$image" "$STORE/$key" "$FS_TOOLS" \
            > "/tmp/prep-$key.log" 2>&1; then
        record "$image" "prepare" false "$(tail -2 /tmp/prep-$key.log | tr '\n"' '  ' | cut -c1-160)"
        echo "  prepare FAILED"
        continue
    fi
    entry=$(find "$STORE/$key" -maxdepth 1 -name 'ref-*' | head -1)
    if [[ -z "$entry" ]]; then
        record "$image" "prepare" false "no entry produced"
        continue
    fi
    record "$image" "prepare" true "$(du -sh "$entry" | cut -f1)"

    if ! timeout 900 "$REPO/target/release/examples/capture_snapshot" "$entry" 1024 \
            > "/tmp/cap-$key.log" 2>&1; then
        record "$image" "capture" false "$(tail -2 /tmp/cap-$key.log | tr '\n"' '  ' | cut -c1-160)"
        echo "  capture FAILED"
        continue
    fi
    record "$image" "capture" true "snapshot written"

    cd "$REPO"
    export SOMA_GENERATION_STORE="$STORE/$key" SOMA_HEAD_DIR=/srv/soma/heads \
           SOMA_ALLOW_UNCERTIFIED_GENERATION=1
    out=$(timeout 300 ./target/release/soma --format json --backend kvm run "$image" -- $cmd 2>/dev/null \
          | python3 -c "
import base64, json, sys
try:
    r = json.loads(sys.stdin.read().strip().splitlines()[-1])
    s = base64.b64decode(r['result']['stdout']['data']).decode() if r.get('result') else ''
    ms = {m['kind']: round(m['elapsed_ns']/1e6,1) for m in r['receipt']['milestones']}
    print(json.dumps({'status': r['status'], 'stdout': s[:200],
                      'tti_ms': ms.get('command_finished'),
                      'error': (r.get('error') or {}).get('code')}))
except Exception as e:
    print(json.dumps({'status':'parse_error','stdout':'','error':str(e)[:80]}))
")
    if grep -q "$expect" <<< "$out"; then
        record "$image" "run" true "$(tr -d '"' <<< "$out" | cut -c1-200)"
        echo "  run OK"
    else
        record "$image" "run" false "$(tr -d '"' <<< "$out" | cut -c1-200)"
        echo "  run FAILED: $out"
    fi
done

echo "=== matrix complete ==="
cat "$OUT"
