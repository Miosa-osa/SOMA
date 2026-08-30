#!/usr/bin/env bash
#
# Builds the statically linked x86_64 Linux guest agent and prints its size, digest, and
# linkage so the Generation compiler can pin the exact artifact.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
readonly TARGET="x86_64-unknown-linux-musl"
readonly BINARY="target/${TARGET}/release/soma-guest-agent"

cd "${REPO_ROOT}"

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        printf 'required command not found: %s\n' "$1" >&2
        return 1
    fi
}

require_command cargo
require_command rustup
require_command file
require_command sha256sum

if ! rustup target list --installed | grep -qx "${TARGET}"; then
    printf 'missing Rust target %s; run: rustup target add %s\n' "${TARGET}" "${TARGET}" >&2
    exit 1
fi

# Optional cargo features, e.g. SOMA_GUEST_AGENT_FEATURES=timing-report to render the
# repair-step timings on the guest console. Empty for the shipped agent.
readonly FEATURES="${SOMA_GUEST_AGENT_FEATURES:-}"

RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static -C strip=symbols" \
    cargo build --locked --release --target "${TARGET}" -p soma-guest-agent \
    --features "${FEATURES}"

if [[ ! -f "${BINARY}" ]]; then
    printf 'expected binary is missing: %s\n' "${BINARY}" >&2
    exit 1
fi

description="$(file -b "${BINARY}")"
if [[ "${description}" != *"statically linked"* ]]; then
    printf 'guest agent is not statically linked: %s\n' "${description}" >&2
    exit 1
fi

if ldd "${BINARY}" >/dev/null 2>&1; then
    printf 'guest agent must not be a dynamic executable\n' >&2
    exit 1
fi

printf 'binary: %s\n' "${BINARY}"
printf 'size:   %s bytes\n' "$(stat -c %s "${BINARY}")"
printf 'sha256: %s\n' "$(sha256sum "${BINARY}" | cut -d ' ' -f 1)"
printf 'file:   %s\n' "${description}"
