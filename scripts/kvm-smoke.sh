#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
readonly TEST_FILE="crates/soma-kvm/tests/kvm_probe.rs"
readonly TEST_NAME="opens_dev_kvm_and_reports_required_capabilities"

cd "${REPO_ROOT}"

if [[ "$(uname -s)" != "Linux" ]]; then
    printf 'KVM smoke requires Linux, found %s\n' "$(uname -s)" >&2
    exit 1
fi

if [[ "$(uname -m)" != "x86_64" ]]; then
    printf 'KVM smoke requires x86_64, found %s\n' "$(uname -m)" >&2
    exit 1
fi

if [[ ! -r /etc/os-release ]]; then
    printf 'KVM smoke requires a readable /etc/os-release\n' >&2
    exit 1
fi

os_id="$(sed -n 's/^ID=//p' /etc/os-release | tr -d '\"')"
os_version="$(sed -n 's/^VERSION_ID=//p' /etc/os-release | tr -d '\"')"
if [[ "$os_id" != "ubuntu" || "$os_version" != "24.04" ]]; then
    printf 'KVM smoke requires Ubuntu 24.04, found %s %s\n' \
        "${os_id:-unknown}" "${os_version:-unknown}" >&2
    exit 1
fi

if [[ ! -e /dev/kvm ]]; then
    printf '/dev/kvm is absent; this host cannot provide KVM evidence\n' >&2
    exit 1
fi

if [[ ! -c /dev/kvm ]]; then
    printf '/dev/kvm exists but is not a character device\n' >&2
    exit 1
fi

if [[ ! -r /dev/kvm || ! -w /dev/kvm ]]; then
    printf '/dev/kvm must be readable and writable by the runner identity\n' >&2
    exit 1
fi

if [[ ! -f "$TEST_FILE" ]]; then
    printf 'canonical KVM smoke target is missing: %s\n' "$TEST_FILE" >&2
    exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
    printf 'cargo is required on the self-hosted KVM runner\n' >&2
    exit 1
fi

cargo metadata --locked --format-version 1 --no-deps >/dev/null
listed_tests="$(cargo test --locked -p soma-kvm --test kvm_probe -- --list)"
if ! printf '%s\n' "$listed_tests" | grep -F -x -q "${TEST_NAME}: test"; then
    printf 'canonical KVM smoke test was not discovered: %s\n' "$TEST_NAME" >&2
    exit 1
fi

cargo test --locked -p soma-kvm --test kvm_probe -- \
    --ignored --exact "$TEST_NAME"
