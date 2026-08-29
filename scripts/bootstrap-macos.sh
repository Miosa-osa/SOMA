#!/usr/bin/env bash

set -euo pipefail

readonly RUNTIME_VERSION="1.3.0"
readonly RUNTIME_SHA256="bd156250cb84061367ed4b0eeef52211b6a825c6e0728a9426e57602ddb089c1"
readonly RUNTIME_PACKAGE="container-${RUNTIME_VERSION}-installer-signed.pkg"
readonly RUNTIME_URL="https://github.com/apple/container/releases/download/${RUNTIME_VERSION}/${RUNTIME_PACKAGE}"

fail() {
    printf 'soma macOS bootstrap: %s\n' "$1" >&2
    exit 1
}

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        fail "required command not found: $1"
    fi
}

if (( $# != 0 )); then
    fail 'this command accepts no arguments'
fi

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]]; then
    fail "requires Apple Silicon macOS, found $(uname -s) $(uname -m)"
fi

require_command curl
require_command pkgutil
require_command shasum

if [[ -z "${HOME:-}" || ! -d "$HOME" ]]; then
    fail 'HOME must identify the current user directory'
fi

runtime_root="${SOMA_MACOS_RUNTIME_ROOT:-${HOME}/Library/Application Support/SOMA/apple-container}"
install_root="${runtime_root}/${RUNTIME_VERSION}"
app_root="${runtime_root}/state"
log_root="${HOME}/Library/Logs/SOMA/apple-container"
runtime_bin="${install_root}/bin/container"

install_runtime() {
    local stage_dir package_path expanded_path observed_sha

    stage_dir="$(mktemp -d "${TMPDIR:-/tmp}/soma-macos-runtime.XXXXXX")"
    package_path="${stage_dir}/${RUNTIME_PACKAGE}"
    expanded_path="${stage_dir}/expanded"
    cleanup_stage() {
        rm -rf -- "$stage_dir"
    }
    trap cleanup_stage RETURN

    printf 'Downloading Apple container %s...\n' "$RUNTIME_VERSION"
    curl \
        --fail \
        --location \
        --proto '=https' \
        --show-error \
        --silent \
        --tlsv1.2 \
        --output "$package_path" \
        "$RUNTIME_URL"

    observed_sha="$(shasum -a 256 "$package_path" | awk '{print $1}')"
    if [[ "$observed_sha" != "$RUNTIME_SHA256" ]]; then
        fail "package digest mismatch: expected ${RUNTIME_SHA256}, found ${observed_sha}"
    fi

    pkgutil --check-signature "$package_path" >/dev/null
    pkgutil --expand-full "$package_path" "$expanded_path"
    if [[ ! -x "${expanded_path}/Payload/bin/container" ]]; then
        fail 'verified package does not contain the expected container executable'
    fi

    mkdir -p "$runtime_root"
    if [[ -e "$install_root" ]]; then
        if [[ ! -x "$runtime_bin" ]]; then
            fail "existing runtime directory is incomplete: ${install_root}"
        fi
        return
    fi
    mv "${expanded_path}/Payload" "$install_root"
}

verify_runtime_version() {
    local reported_version

    reported_version="$("$runtime_bin" --version)"
    case "$reported_version" in
        "container CLI version ${RUNTIME_VERSION} "*)
            ;;
        *)
            fail "unexpected runtime version: ${reported_version}"
            ;;
    esac
}

if [[ ! -x "$runtime_bin" ]]; then
    install_runtime
fi
verify_runtime_version

mkdir -p "$app_root" "$log_root"
printf 'Starting the user-owned VM runtime...\n'
"$runtime_bin" system start \
    --install-root "$install_root" \
    --app-root "$app_root" \
    --log-root "$log_root" \
    --enable-kernel-install \
    --timeout 120

status_json="$("$runtime_bin" system status --format json)"
if ! printf '%s\n' "$status_json" | grep -Eq '"status"[[:space:]]*:[[:space:]]*"running"'; then
    fail 'runtime service did not report running status'
fi

printf 'SOMA macOS runtime is ready.\n'
printf 'Runtime executable: %s\n' "$runtime_bin"
