#!/usr/bin/env bash
#
# Builds the host bundle: what a Linux x86_64 KVM host runs, and the guest agent its Generations
# embed.
#
#   SOMA_RELEASE_TAG=v1.0.0 scripts/package-host-release.sh
#
# The bundle also holds the template compiler and boot-snapshot capture tool.
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
cargo_home="${CARGO_HOME:-${HOME}/.cargo}"
remap="--remap-path-prefix=${REPO_ROOT}=/soma --remap-path-prefix=${cargo_home}=/cargo"
RUSTFLAGS="${RUSTFLAGS:+${RUSTFLAGS} }${remap}" "$REPO_ROOT/scripts/build-identity.sh" \
    cargo build --release --locked --target "$HOST_TARGET" \
    -p soma-generation --example prepare_from_template \
    -p soma-local --example capture_snapshot --example install_generation

tools_dir="target/${HOST_TARGET}/release/examples"
exec "$REPO_ROOT/scripts/package-binaries.sh" host "$HOST_TARGET" \
    soma-api:soma-api \
    soma-hostd:soma-hostd \
    soma-netd:soma-netd \
    soma-vmm:soma-vmm \
    "${tools_dir}/prepare_from_template=prepare_from_template" \
    "${tools_dir}/capture_snapshot=capture_snapshot" \
    "${tools_dir}/install_generation=install_generation" \
    "${AGENT}=soma-guest-agent"
