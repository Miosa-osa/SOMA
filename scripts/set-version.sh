#!/usr/bin/env bash
#
# Moves the whole workspace to one new release version, the first step of cutting a release.
#
#   scripts/set-version.sh 1.0.0-alpha.2
#
# It rewrites VERSION, the `[workspace.package]` version, every `version = "=OLD"` requirement
# between workspace crates, and the workspace entries of Cargo.lock, then runs the version
# contract. It reads no network, commits nothing, and tags nothing: the change is reviewed as a
# pull request like any other, and the tag is cut from the merged commit.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
readonly SEMVER_PATTERN='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$'
# shellcheck source=scripts/release/common.sh
source "${REPO_ROOT}/scripts/release/common.sh"
cd "$REPO_ROOT"

(( $# == 1 )) || fail 'usage: set-version.sh NEW_VERSION'
new_version="$1"
[[ "$new_version" =~ $SEMVER_PATTERN ]] ||
    fail "not a Semantic Versioning release version without build metadata: ${new_version}"
require_command cargo
"$REPO_ROOT/scripts/check-version.sh" >/dev/null
old_version="$(tr -d '\r\n' < VERSION)"
[[ "$new_version" != "$old_version" ]] || fail "the workspace is already at ${old_version}"

# Literal replacement of whole fields only: the old version is matched as text, never as a
# pattern, and only on the lines that carry the workspace version.
replace_lines() {
    local file="$1"
    local old_line="$2"
    local new_line="$3"
    local temporary="${file}.set-version"

    awk -v old="$old_line" -v new="$new_line" '{
        line = $0
        position = index(line, old)
        if (position > 0) {
            line = substr(line, 1, position - 1) new substr(line, position + length(old))
        }
        print line
    }' "$file" > "$temporary"
    mv "$temporary" "$file"
}

printf '%s\n' "$new_version" > VERSION
grep -qx "version = \"${old_version}\"" Cargo.toml ||
    fail 'Cargo.toml has no workspace package version line'
replace_lines Cargo.toml "version = \"${old_version}\"" "version = \"${new_version}\""
pinned=0
while IFS= read -r -d '' manifest; do
    if grep -qF "version = \"=${old_version}\"" "$manifest"; then
        replace_lines "$manifest" "version = \"=${old_version}\"" "version = \"=${new_version}\""
        pinned=$((pinned + 1))
    fi
done < <(find crates -mindepth 2 -maxdepth 2 -name Cargo.toml -print0)

cargo update --workspace --offline --quiet
"$REPO_ROOT/scripts/check-version.sh"
printf 'workspace moved from %s to %s (%d crate manifests carried workspace requirements)\n' \
    "$old_version" "$new_version" "$pinned"
