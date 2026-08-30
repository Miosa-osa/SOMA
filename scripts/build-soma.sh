#!/usr/bin/env bash
#
# Builds the three SOMA artifacts a KVM engine host needs, from the current checkout.
#
#   1. the soma command line               cargo build --release
#   2. the static x86_64 guest agent       scripts/build-guest-agent.sh
#   3. the pinned PVH guest kernel          kernel/build.sh   (about 60 seconds of compile)
#
# Run it from the repository root after setup-host.sh. It prints the path and digest of each
# artifact at the end so the prepare and run steps can reference them.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
cd "$REPO_ROOT"

readonly MUSL_TARGET="x86_64-unknown-linux-musl"
readonly SOMA_BIN="target/release/soma"
readonly AGENT_BIN="target/${MUSL_TARGET}/release/soma-guest-agent"

log() { printf '\n==> %s\n' "$1"; }

build_cli() {
    log "building the soma command line"
    cargo build --release -p soma-cli
}

build_agent() {
    log "building the static guest agent"
    ./scripts/build-guest-agent.sh >/dev/null
}

build_kernel() {
    log "building the pinned guest kernel"
    if compgen -G "kernel/out/vmlinux-*-soma-v1" >/dev/null; then
        printf '  kernel already built, skipping; delete kernel/out to rebuild\n'
        return
    fi
    ./kernel/build.sh
}

digest() { sha256sum "$1" 2>/dev/null | cut -c1-16; }

report() {
    log "artifacts"
    local kernel
    kernel="$(compgen -G "kernel/out/vmlinux-*-soma-v1" | head -1 || true)"
    local ok=1
    for pair in "soma:$SOMA_BIN" "guest-agent:$AGENT_BIN" "kernel:${kernel:-MISSING}"; do
        local name="${pair%%:*}" path="${pair#*:}"
        if [[ -f "$path" ]]; then
            printf '  ok    %-12s %s  (%s)\n' "$name" "$path" "$(digest "$path")"
        else
            printf '  FAIL  %-12s not found\n' "$name"
            ok=0
        fi
    done
    [[ "$ok" -eq 1 ]] || { printf '\nBuild incomplete.\n' >&2; exit 1; }
    printf '\nAll three artifacts built. Next: build-fs-tools.sh, then prepare-generation.sh.\n'
}

main() {
    build_cli
    build_agent
    build_kernel
    report
}

main "$@"
