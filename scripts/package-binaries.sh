#!/usr/bin/env bash
#
# Builds one release bundle of SOMA binaries for one target and describes every binary in it.
#
#   SOMA_RELEASE_TAG=v1.0.0 scripts/package-binaries.sh KIND TARGET ITEM...
#
# KIND is `client` (archive soma-TAG-TARGET.tar.gz) or `host` (soma-host-TAG-TARGET.tar.gz).
# An ITEM is PACKAGE:BINARY, built with `cargo build --release --locked --target TARGET` through
# scripts/build-identity.sh, or FILE=BINARY, a binary already built from this tree, such as the
# static guest agent. Every binary must run here: its own `--version` report is what the bundle's
# record of it is made from, so the record and the binary cannot disagree.
#
# The output directory (SOMA_RELEASE_OUTPUT, default target/release-parts/TAG/KIND-TARGET) holds
# the archive, ARCHIVE_BASE.binaries.json with one record per binary, and SHA256SUMS over both.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
# shellcheck source=scripts/release/common.sh
source "${REPO_ROOT}/scripts/release/common.sh"
cd "$REPO_ROOT"

kind="${1:-}"
release_target="${2:-}"
shift 2 || fail 'usage: package-binaries.sh KIND TARGET ITEM...'
(( $# > 0 )) || fail 'usage: package-binaries.sh KIND TARGET ITEM...'
release_tag="${SOMA_RELEASE_TAG:-}"
allow_dirty="${SOMA_RELEASE_ALLOW_DIRTY:-0}"
case "$kind" in
    client) bundle_name="soma-${release_tag}-${release_target}" ;;
    host) bundle_name="soma-host-${release_tag}-${release_target}" ;;
    *) fail 'KIND must be client or host' ;;
esac
[[ "$release_tag" =~ ^v[0-9A-Za-z.+-]+$ ]] || fail 'SOMA_RELEASE_TAG must be a tag such as v1.0.0'
[[ "$release_target" =~ ^[0-9A-Za-z_.-]+$ ]] || fail 'TARGET must be a safe Rust target triple'
[[ "$allow_dirty" == 0 || "$allow_dirty" == 1 ]] || fail 'SOMA_RELEASE_ALLOW_DIRTY must be 0 or 1'
for tool in cargo git jq rustc tar gzip; do
    require_command "$tool"
done
"$REPO_ROOT/scripts/check-version.sh" >/dev/null
release_version="$(tr -d '\r\n' < VERSION)"
[[ "$release_tag" == "v${release_version}" ]] ||
    fail "release tag ${release_tag} does not match VERSION ${release_version}"
commit="$(git rev-parse --verify 'HEAD^{commit}')" || fail 'packaging requires a committed revision'
if [[ "$allow_dirty" == 0 && -n "$(git status --porcelain=v1 --untracked-files=normal)" ]]; then
    fail 'packaging requires a clean working tree'
fi
output_dir="${SOMA_RELEASE_OUTPUT:-${REPO_ROOT}/target/release-parts/${release_tag}/${kind}-${release_target}}"
[[ ! -e "$output_dir" ]] || fail "release output already exists: ${output_dir}"

packages=()
for item in "$@"; do
    if [[ "$item" == *:* && "$item" != *=* ]]; then
        packages+=(-p "${item%%:*}")
    fi
done
if (( ${#packages[@]} > 0 )); then
    # Paths of this machine never reach a binary: the checkout and the Cargo home are remapped,
    # so two machines building one commit with one toolchain produce the same bytes.
    cargo_home="${CARGO_HOME:-${HOME}/.cargo}"
    remap="--remap-path-prefix=${REPO_ROOT}=/soma --remap-path-prefix=${cargo_home}=/cargo"
    RUSTFLAGS="${RUSTFLAGS:+${RUSTFLAGS} }${remap}" "$REPO_ROOT/scripts/build-identity.sh" \
        cargo build --release --locked --target "$release_target" "${packages[@]}"
fi

stage_dir="$(mktemp -d "${TMPDIR:-/tmp}/soma-binaries.XXXXXX")"
cleanup() {
    rm -rf -- "$stage_dir"
}
trap cleanup EXIT
bundle_dir="$stage_dir/$bundle_name"
delivery_dir="$stage_dir/delivery"
mkdir -p "$bundle_dir" "$delivery_dir"
names=()
for item in "$@"; do
    if [[ "$item" == *=* ]]; then
        source_file="${item%%=*}"
        name="${item#*=}"
    else
        name="${item#*:}"
        source_file="target/${release_target}/release/${name}"
    fi
    [[ "$name" =~ ^[0-9A-Za-z_.-]+$ ]] || fail "binary names must be safe base names: ${name}"
    [[ -f "$source_file" ]] || fail "built binary is missing: ${source_file}"
    cp "$source_file" "$bundle_dir/$name"
    chmod 0755 "$bundle_dir/$name"
    names+=("$name")
done
cp LICENSE NOTICE "$bundle_dir/"
{
    printf 'release_tag=%s\n' "$release_tag"
    printf 'target=%s\n' "$release_target"
    printf 'revision=%s\n' "$commit"
    rustc --version
    cargo --version
} > "$bundle_dir/BUILD-INFO.txt"
write_checksum_manifest "$bundle_dir" SHA256SUMS BUILD-INFO.txt LICENSE NOTICE "${names[@]}"

archive_name="${bundle_name}.tar.gz"
tar_options=()
if tar --version 2>/dev/null | grep -q 'GNU tar'; then
    # Same commit, same archive bytes: ordered names, the commit's own time, no build user.
    tar_options=(--sort=name --owner=0 --group=0 --numeric-owner
        "--mtime=@$(git show -s --format=%ct HEAD)")
fi
# The guarded expansion keeps an empty option list legal under `set -u` in bash 3, as on macOS.
tar -C "$stage_dir" -cf - ${tar_options[@]+"${tar_options[@]}"} "$bundle_name" |
    gzip -n -9 > "$delivery_dir/$archive_name"

# One record per binary, read from the binary's own report.
records_name="${bundle_name}.binaries.json"
"$REPO_ROOT/scripts/release/describe-binaries.sh" \
    "$bundle_dir" "$archive_name" "$bundle_name" "$release_version" "$commit" "$allow_dirty" \
    "${names[@]}" > "$delivery_dir/$records_name"
write_checksum_manifest "$delivery_dir" SHA256SUMS "$archive_name" "$records_name"

"$REPO_ROOT/scripts/verify-release-artifacts.sh" bundle \
    "$delivery_dir" "$release_tag" "$release_target" "$kind" "${names[@]}"
mkdir -p "$(dirname "$output_dir")"
mv "$delivery_dir" "$output_dir"
printf '%s bundle ready: %s\n' "$kind" "$output_dir"
