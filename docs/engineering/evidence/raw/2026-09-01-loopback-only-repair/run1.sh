#!/usr/bin/env bash
source $HOME/.cargo/env
cd /srv/soma/agent-macloop/repo
exec ./scripts/reproduce.sh --root /srv/soma/agent-macloop/root   --memory-mib 256 --storage-mib 512 --samples 10 busybox:stable-musl
