#!/usr/bin/env bash
#
# Builds a whole release from a fixture repository with the real packagers and verifier: a
# source part, a client bundle of two binaries that print real `--version` reports, and the
# assembled release with its manifest. Then it tampers with the result and expects refusals.
# The host bundle needs the static guest agent and Linux, so `check.sh release` covers it.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIR
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd -P)"
readonly REPO_ROOT
# shellcheck source=scripts/release/common.sh
source "${REPO_ROOT}/scripts/release/common.sh"

for tool in cargo git jq rustc tar gzip; do
    require_command "$tool"
done

test_root="$(mktemp -d "${TMPDIR:-/tmp}/soma-package-test.XXXXXX")"
cleanup() {
    find "$test_root" -depth -delete
}
trap cleanup EXIT

version="$(tr -d '\r\n' < "$REPO_ROOT/VERSION")"
tag="v${version}"
target="$(rustc -vV | sed -n 's/^host: //p')"
fixture="$test_root/repository"
parts="$test_root/parts"
release="$test_root/release"
mkdir -p "$fixture/crates/fixture/src/bin" "$fixture/scripts/release"
cp "$REPO_ROOT/LICENSE" "$REPO_ROOT/NOTICE" "$REPO_ROOT/rust-toolchain.toml" "$fixture/"
for script in check-version.sh build-identity.sh package-binaries.sh package-source-release.sh \
    assemble-release.sh verify-release-artifacts.sh release/common.sh \
    release/describe-binaries.sh release/verify-release.sh; do
    cp "$REPO_ROOT/scripts/$script" "$fixture/scripts/$script"
done
printf '%s\n' "$version" > "$fixture/VERSION"
printf 'target/\n' > "$fixture/.gitignore"
printf '%s\n' \
    '[workspace]' \
    'members = ["crates/fixture"]' \
    'resolver = "3"' \
    '' \
    '[workspace.package]' \
    "version = \"${version}\"" \
    > "$fixture/Cargo.toml"
printf '%s\n' \
    '[package]' \
    'name = "fixture"' \
    'version.workspace = true' \
    'edition = "2024"' \
    'publish = false' \
    '' \
    '[dependencies]' \
    "soma-version = { path = \"${REPO_ROOT}/crates/soma-version\" }" \
    > "$fixture/crates/fixture/Cargo.toml"
for binary in soma soma-mcp; do
    printf '%s\n' \
        'fn main() {' \
        '    if soma_version::version_requested(std::env::args_os()) {' \
        '        let protocols = [soma_version::Protocol::new("fixture", 7)];' \
        '        let speaks = soma_version::Speaks { api: None, protocols: &protocols };' \
        "        print!(\"{}\", soma_version::report(\"${binary}\", &soma_version::build!(), &speaks));" \
        '    }' \
        '}' \
        > "$fixture/crates/fixture/src/bin/${binary}.rs"
done

(
    cd "$fixture"
    cargo generate-lockfile --offline --quiet
    git init -q
    git config user.email 'soma-release-test@example.invalid'
    git config user.name 'SOMA release test'
    git add .
    git commit -qm 'release fixture'
    export SOMA_RELEASE_TAG="$tag"
    SOMA_RELEASE_OUTPUT="$parts/source" ./scripts/package-source-release.sh
    SOMA_RELEASE_OUTPUT="$parts/client" \
        ./scripts/package-binaries.sh client "$target" fixture:soma fixture:soma-mcp
    ./scripts/assemble-release.sh "$release" "$parts/source" "$parts/client"
)
commit="$(git -C "$fixture" rev-parse HEAD)"
manifest="$release/soma-${tag}-manifest.json"

jq -e --arg commit "$commit" --arg tag "$tag" --arg target "$target" '
    .schema == "soma.release-manifest.v1" and .tag == $tag and .commit == $commit
    and (.artifacts | length) == 2 and (.binaries | length) == 2
    and all(.binaries[]; .commit == $commit and .reports_commit and .source == "clean"
        and .target == $target and .protocols == {"fixture": 7} and .api_version == null)
    ' "$manifest" >/dev/null || fail "the manifest does not describe the fixture: $(cat "$manifest")"
[[ -f "$release/soma-${tag}-source.tar.gz" && -f "$release/soma-${tag}-${target}.tar.gz" ]] ||
    fail 'the release does not hold both archives'

expect_refusal() {
    local description="$1"
    local damaged="$2"
    if "$fixture/scripts/verify-release-artifacts.sh" release "$damaged" "$tag" \
        > "$test_root/refusal.log" 2>&1; then
        fail "the verifier accepted ${description}"
    fi
}

# A manifest that lies about a binary's size is refused even with a consistent SHA256SUMS.
lying="$test_root/lying"
cp -R "$release" "$lying"
jq '.binaries[0].size += 1' "$manifest" > "$lying/soma-${tag}-manifest.json"
(cd "$lying" && find . -mindepth 1 -maxdepth 1 -type f ! -name SHA256SUMS -exec basename {} \; |
    LC_ALL=C sort > "$test_root/names")
names=()
while IFS= read -r name; do
    names+=("$name")
done < "$test_root/names"
write_checksum_manifest "$lying" SHA256SUMS "${names[@]}"
expect_refusal 'a manifest with a wrong binary size' "$lying"

# An artifact changed after the checksums were written is refused.
changed="$test_root/changed"
cp -R "$release" "$changed"
printf 'x' >> "$changed/soma-${tag}-source.tar.gz"
expect_refusal 'an artifact changed after signing its checksum' "$changed"

# A file nobody listed is refused.
extra="$test_root/extra"
cp -R "$release" "$extra"
printf 'extra\n' > "$extra/unlisted.txt"
expect_refusal 'an unlisted file' "$extra"

printf 'release packager regression test passed\n'
