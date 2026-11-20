#!/usr/bin/env bash
#
# Runs one command with the source identity of this checkout in its environment, so every SOMA
# binary the command builds reports the commit it was built from.
#
#   scripts/build-identity.sh cargo build --release --locked -p soma-api
#   scripts/build-identity.sh --print        # NAME=value lines, for a shell or GITHUB_ENV
#
# It sets three variables, which `soma_version::build!()` reads in each binary's own crate:
#
#   SOMA_BUILD_COMMIT       the full commit
#   SOMA_BUILD_COMMIT_DATE  the commit's own committer date in UTC, YYYY-MM-DDTHH:MM:SSZ
#   SOMA_BUILD_SOURCE       clean, or dirty when any file Git does not ignore differs from it
#
# Only the local repository is read: no network and no clock, so one clean commit always yields
# the same three values and the same binaries. Outside a Git checkout of this repository the
# variables are removed rather than guessed, and the binaries report the commit as unknown.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
readonly COMMIT_PATTERN='^([0-9a-f]{40}|[0-9a-f]{64})$'
readonly DATE_PATTERN='^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$'

unset SOMA_BUILD_COMMIT SOMA_BUILD_COMMIT_DATE SOMA_BUILD_SOURCE

# Fills the three variables, or returns non-zero and leaves them unset.
read_identity() {
    local top commit date changes

    command -v git >/dev/null 2>&1 || return 1
    top="$(git -C "$REPO_ROOT" rev-parse --show-toplevel 2>/dev/null)" || return 1
    # A checkout nested inside another repository must not borrow that repository's commit.
    [[ "$(cd "$top" && pwd -P)" == "$REPO_ROOT" ]] || return 1
    commit="$(git -C "$REPO_ROOT" rev-parse --verify --quiet 'HEAD^{commit}')" || return 1
    [[ "$commit" =~ $COMMIT_PATTERN ]] || return 1
    date="$(TZ=UTC0 git -C "$REPO_ROOT" show -s --no-show-signature \
        --date='format-local:%Y-%m-%dT%H:%M:%SZ' --format=%cd HEAD)" || return 1
    [[ "$date" =~ $DATE_PATTERN ]] || return 1
    # Optional locks are off so reading the status never rewrites the index.
    changes="$(git --no-optional-locks -C "$REPO_ROOT" status --porcelain=v1 \
        --untracked-files=normal)" || return 1

    SOMA_BUILD_COMMIT="$commit"
    SOMA_BUILD_COMMIT_DATE="$date"
    if [[ -n "$changes" ]]; then
        SOMA_BUILD_SOURCE=dirty
    else
        SOMA_BUILD_SOURCE=clean
    fi
    export SOMA_BUILD_COMMIT SOMA_BUILD_COMMIT_DATE SOMA_BUILD_SOURCE
}

if (( $# == 0 )); then
    printf 'usage: build-identity.sh --print | COMMAND [ARGUMENT...]\n' >&2
    exit 2
fi

identified=1
if ! read_identity; then
    identified=0
    unset SOMA_BUILD_COMMIT SOMA_BUILD_COMMIT_DATE SOMA_BUILD_SOURCE
fi

if [[ "$1" == "--print" ]]; then
    (( $# == 1 )) || { printf 'usage: build-identity.sh --print\n' >&2; exit 2; }
    if (( identified == 1 )); then
        printf 'SOMA_BUILD_COMMIT=%s\n' "$SOMA_BUILD_COMMIT"
        printf 'SOMA_BUILD_COMMIT_DATE=%s\n' "$SOMA_BUILD_COMMIT_DATE"
        printf 'SOMA_BUILD_SOURCE=%s\n' "$SOMA_BUILD_SOURCE"
    fi
    exit 0
fi

if (( identified == 0 )); then
    printf 'build-identity.sh: no Git checkout of this repository; binaries report unknown\n' >&2
fi
exec "$@"
