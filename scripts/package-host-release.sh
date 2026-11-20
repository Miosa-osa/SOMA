#!/usr/bin/env bash
#
# Builds the host bundle: what a Linux x86_64 KVM host runs, and the guest agent its Generations
# embed.
#
#   SOMA_RELEASE_TAG=v1.0.0 scripts/package-host-release.sh
#
# The bundle holds soma-api, soma-hostd, soma-netd, soma-vmm, and the static soma-guest-agent.
# The agent is built by scripts/build-guest-agent.sh, exactly as a Generation build takes it. The
# bundle, its binary records, and its checksums land in SOMA_RELEASE_OUTPUT, by default
# target/release-parts/TAG/host-x86_64-unknown-linux-gnu.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
readonly HOST_TARGET="x86_64-unknown-linux-gnu"
readonly AGENT="target/x86_64-unknown-linux-musl/release/soma-guest-agent"
# shellcheck source=scripts/release/common.sh
source "${REPO_ROOT}/scripts/release/common.sh"
cd "$REPO_ROOT"

if [[ "$(uname -s)" != Linux || "$(uname -m)" != x86_64 ]]; then
    fail "the host bundle is built on Linux x86_64, found $(uname -s) $(uname -m)"
fi

"$REPO_ROOT/scripts/build-guest-agent.sh"
exec "$REPO_ROOT/scripts/package-binaries.sh" host "$HOST_TARGET" \
    soma-api:soma-api \
    soma-hostd:soma-hostd \
    soma-netd:soma-netd \
    soma-vmm:soma-vmm \
    "${AGENT}=soma-guest-agent"
