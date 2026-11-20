#!/usr/bin/env bash
#
# Builds the source part of a release: the archive of the committed tree, and its checksum.
#
#   SOMA_RELEASE_TAG=v1.0.0 scripts/package-source-release.sh
#
# `git archive` stamps every entry with the commit's own time and gzip writes no name or time,
# so one commit always gives the same archive bytes. The part lands in SOMA_RELEASE_OUTPUT, by
# default target/release-parts/TAG/source.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
# shellcheck source=scripts/release/common.sh
source "${REPO_ROOT}/scripts/release/common.sh"
cd "$REPO_ROOT"

release_tag="${SOMA_RELEASE_TAG:-}"
[[ "$release_tag" =~ ^v[0-9A-Za-z.+-]+$ ]] || fail 'SOMA_RELEASE_TAG must be a tag such as v1.0.0'
require_command git
require_command gzip
"$REPO_ROOT/scripts/check-version.sh" >/dev/null
[[ "$release_tag" == "v$(tr -d '\r\n' < VERSION)" ]] ||
    fail "release tag ${release_tag} does not match VERSION"
git rev-parse --verify 'HEAD^{commit}' >/dev/null 2>&1 || fail 'a source part needs a commit'
output_dir="${SOMA_RELEASE_OUTPUT:-${REPO_ROOT}/target/release-parts/${release_tag}/source}"
[[ ! -e "$output_dir" ]] || fail "release output already exists: ${output_dir}"

archive_name="soma-${release_tag}-source.tar.gz"
mkdir -p "$output_dir"
git archive --format=tar --prefix="soma-${release_tag}/" HEAD |
    gzip -n -9 > "${output_dir}/${archive_name}"
write_checksum_manifest "$output_dir" SHA256SUMS "$archive_name"
"$REPO_ROOT/scripts/verify-release-artifacts.sh" source "$output_dir" "$release_tag"
