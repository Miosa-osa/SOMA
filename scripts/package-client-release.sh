#!/usr/bin/env bash
#
# Builds the client bundle for one target: the `soma` command line and the `soma-mcp` server.
#
#   SOMA_RELEASE_TAG=v1.0.0 SOMA_RELEASE_TARGET=x86_64-unknown-linux-gnu \
#       SOMA_RELEASE_CLI_BINARY=soma SOMA_RELEASE_MCP_BINARY=soma-mcp \
#       scripts/package-client-release.sh
#
# The binary names carry the platform suffix, `soma.exe` on Windows. The bundle, its binary
# records, and its checksums land in SOMA_RELEASE_OUTPUT, by default
# target/release-parts/TAG/client-TARGET; scripts/package-binaries.sh does the work.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
# shellcheck source=scripts/release/common.sh
source "${REPO_ROOT}/scripts/release/common.sh"

release_target="${SOMA_RELEASE_TARGET:-}"
cli_binary="${SOMA_RELEASE_CLI_BINARY:-}"
mcp_binary="${SOMA_RELEASE_MCP_BINARY:-}"
[[ "$release_target" =~ ^[0-9A-Za-z_.-]+$ ]] ||
    fail 'SOMA_RELEASE_TARGET must be a safe Rust target triple'
for binary in "$cli_binary" "$mcp_binary"; do
    [[ "$binary" =~ ^[0-9A-Za-z_.-]+$ ]] || fail 'release binary names must be safe base names'
done
[[ "$cli_binary" != "$mcp_binary" ]] || fail 'CLI and MCP release binary names must be distinct'

exec "$REPO_ROOT/scripts/package-binaries.sh" client "$release_target" \
    "soma-cli:${cli_binary}" "soma-mcp:${mcp_binary}"
