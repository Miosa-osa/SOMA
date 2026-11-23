#!/usr/bin/env bash
set -uo pipefail
source $HOME/.cargo/env
cd /srv/soma/agent-macloop/repo
export SOMA_GENERATION_STORE=/srv/soma/agent-macloop/root/store/busybox-stable-musl-1-256-512
export SOMA_HEAD_DIR=/srv/soma/agent-macloop/root/heads
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1
OUT=/srv/soma/agent-macloop/samples
mkdir -p "$OUT"
CMD="ls /sys/class/net; echo --net-devices-above--; ip -o link; echo --routes--; ip route; echo --repair--; ifconfig lo | head -4; (echo soma-lo-ok | nc -l -p 15432 &); sleep 1; nc 127.0.0.1 15432; echo --after--; ifconfig lo | grep -E \"RX packets|TX packets\""
for i in $(seq 0 9); do
  ./target/release/soma --format json --backend kvm run --vcpus 1 --memory-mib 256 --storage-mib 512 \
    busybox:stable-musl -- /bin/sh -c "$CMD" > "$OUT/lo-$i.json" 2> "$OUT/lo-$i.err"
  echo "sample $i exit=$?"
done
