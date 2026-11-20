#!/usr/bin/env bash
#
# Builds the native release of SOMA_RELEASE_TAG on Linux x86_64 and verifies it end to end.
#
#   SOMA_RELEASE_TAG=v1.0.0 scripts/package-release.sh
#
#   1. the source archive of the commit          target/release-parts/TAG/source
#   2. the host bundle                            target/release-parts/TAG/host-x86_64-unknown-linux-gnu
#   3. the Linux x86_64 client bundle             target/release-parts/TAG/client-x86_64-unknown-linux-gnu
#   4. the assembled release and its manifest     target/release-bundles/TAG
#
# The release workflow builds the macOS and Windows client bundles on their own runners and
# assembles every part again, so this script alone proves the procedure on one Linux host.
# SOMA_RELEASE_PARTS_ONLY=1 stops after step 3, for the workflow's own assembly.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
# shellcheck source=scripts/release/common.sh
source "${REPO_ROOT}/scripts/release/common.sh"
cd "${REPO_ROOT}"

release_tag="${SOMA_RELEASE_TAG:-}"
allow_dirty="${SOMA_RELEASE_ALLOW_DIRTY:-0}"
parts_only="${SOMA_RELEASE_PARTS_ONLY:-0}"
[[ "$release_tag" =~ ^v[0-9A-Za-z.+-]+$ ]] ||
    fail 'SOMA_RELEASE_TAG must be a SemVer tag such as v1.0.0 or v1.0.0-alpha.1'
[[ "$allow_dirty" == 0 || "$allow_dirty" == 1 ]] || fail 'SOMA_RELEASE_ALLOW_DIRTY must be 0 or 1'
for tool in cargo git gzip jq rustc tar; do
    require_command "$tool"
done
"${REPO_ROOT}/scripts/check-version.sh" >/dev/null
release_version="$(tr -d '\r\n' < VERSION)"
[[ "$release_tag" == "v${release_version}" ]] ||
    fail "VERSION is ${release_version}, expected ${release_tag#v}"
git rev-parse --verify 'HEAD^{commit}' >/dev/null 2>&1 ||
    fail 'release packaging requires a committed revision'
if [[ "$allow_dirty" == 0 && -n "$(git status --porcelain=v1 --untracked-files=normal)" ]]; then
    fail 'release packaging requires a clean working tree'
fi

parts_dir="${SOMA_RELEASE_PARTS:-${REPO_ROOT}/target/release-parts/${release_tag}}"
output_dir="${SOMA_RELEASE_OUTPUT:-${REPO_ROOT}/target/release-bundles/${release_tag}}"
[[ ! -e "$parts_dir" ]] || fail "release parts already exist: ${parts_dir}"
[[ "$parts_only" == 1 || ! -e "$output_dir" ]] || fail "release output already exists: ${output_dir}"

# 1. The source archive of the commit.
SOMA_RELEASE_OUTPUT="${parts_dir}/source" "${REPO_ROOT}/scripts/package-source-release.sh"

# 2. and 3. The binaries this host can build and run.
SOMA_RELEASE_OUTPUT="${parts_dir}/host-x86_64-unknown-linux-gnu" \
    "${REPO_ROOT}/scripts/package-host-release.sh"
SOMA_RELEASE_OUTPUT="${parts_dir}/client-x86_64-unknown-linux-gnu" \
    SOMA_RELEASE_TARGET=x86_64-unknown-linux-gnu \
    SOMA_RELEASE_CLI_BINARY=soma \
    SOMA_RELEASE_MCP_BINARY=soma-mcp \
    "${REPO_ROOT}/scripts/package-client-release.sh"
if [[ "$parts_only" == 1 ]]; then
    printf 'release parts ready: %s\n' "$parts_dir"
    exit 0
fi

# 4. One directory to publish, with its manifest and its checksums, verified as a deployer would.
"${REPO_ROOT}/scripts/assemble-release.sh" "$output_dir" "$parts_dir"/*
printf 'release bundle ready: %s\n' "$output_dir"
