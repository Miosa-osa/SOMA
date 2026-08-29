#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIR
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd -P)"
readonly REPO_ROOT

cd "${REPO_ROOT}"

log() {
    printf '\n==> %s\n' "$1"
}

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        printf 'required command not found: %s\n' "$1" >&2
        return 1
    fi
}

check_workspace() {
    require_command cargo
    require_command rustc

    if [[ ! -f Cargo.toml ]]; then
        printf 'Cargo.toml is missing from the repository root\n' >&2
        return 1
    fi

    if [[ ! -f Cargo.lock ]]; then
        printf 'Cargo.lock must be committed for locked, auditable builds\n' >&2
        return 1
    fi

    cargo metadata --locked --format-version 1 --no-deps >/dev/null
}

check_repository_contract() {
    log "version contract"
    "${SCRIPT_DIR}/check-version.sh"

    log "architecture rules"
    "${SCRIPT_DIR}/check-architecture.sh"

    log "workflow policy"
    "${SCRIPT_DIR}/check-workflows.sh"

    log "shell scripts"
    require_command shellcheck
    local shellcheck_failed=0
    while IFS= read -r -d '' script; do
        if ! shellcheck -x "$script"; then
            shellcheck_failed=1
        fi
    done < <(find scripts -type f -name '*.sh' -print0)

    if (( shellcheck_failed != 0 )); then
        return 1
    fi
}

check_format() {
    log "rustfmt"
    cargo fmt --all --check
}

check_benchmark_harness() {
    require_command python3

    log "benchmark harness tests"
    PYTHONDONTWRITEBYTECODE=1 python3 -W error::ResourceWarning \
        -m unittest discover -s benchmarks/tests -p 'test_*.py' -v

    log "benchmark harness syntax"
    PYTHONDONTWRITEBYTECODE=1 python3 -m compileall -q -f benchmarks
}

check_portable_rust() {
    check_workspace
    check_format
    check_benchmark_harness

    log "portable clippy"
    cargo clippy --workspace --all-targets --locked -- -D warnings

    log "portable tests"
    cargo test --workspace --all-targets --locked

    log "portable documentation tests"
    cargo test --workspace --doc --locked
}

check_linux_rust() {
    if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
        printf 'linux profile requires Linux x86_64, found %s %s\n' \
            "$(uname -s)" "$(uname -m)" >&2
        return 1
    fi

    check_workspace
    check_format
    check_benchmark_harness

    log "Linux clippy"
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

    log "Linux tests without KVM claims"
    cargo test --workspace --all-targets --all-features --locked

    log "Linux documentation tests"
    cargo test --workspace --doc --all-features --locked

    log "Linux build"
    cargo build --workspace --all-targets --all-features --locked
}

check_security_tools() {
    check_workspace
    require_command cargo-deny
    require_command cargo-audit
    require_command typos
    require_command actionlint
    require_command zizmor
    require_command gitleaks

    log "dependency licenses, sources, bans, and advisories"
    cargo deny check --deny warnings advisories bans licenses sources

    log "RustSec vulnerability audit"
    cargo audit --deny warnings

    log "spelling"
    typos

    log "GitHub Actions syntax"
    actionlint

    log "GitHub Actions security"
    zizmor --pedantic --offline .

    log "working tree secret scan"
    gitleaks dir --no-banner --redact=100 .

    if git rev-parse --verify HEAD >/dev/null 2>&1; then
        log "Git history secret scan"
        gitleaks git --no-banner --redact=100 --log-opts="--all" .
    else
        printf 'Git history scan skipped because the repository has no commit yet\n'
    fi
}

usage() {
    cat <<'EOF'
usage: scripts/check.sh [all|portable|linux|security|architecture|release|kvm]

  all           Run the native Rust profile, repository policy, and security tools.
  portable      Run format, lint, and platform-neutral tests.
  linux         Run Ubuntu x86_64 format, lint, test, and build checks.
  security      Run dependency, workflow, spelling, and secret checks.
  architecture  Run source-size, dash, workflow, and shell checks.
  release       Validate, package, and checksum SOMA_RELEASE_TAG.
  kvm           Run the real KVM smoke test after strict host detection.
EOF
}

main() {
    local profile="${1:-all}"

    if (( $# > 1 )); then
        usage >&2
        return 2
    fi

    case "$profile" in
        all)
            check_repository_contract
            if [[ "$(uname -s)" == "Linux" && "$(uname -m)" == "x86_64" ]]; then
                check_linux_rust
            else
                check_portable_rust
            fi
            check_security_tools
            ;;
        portable)
            check_repository_contract
            check_portable_rust
            ;;
        linux)
            check_repository_contract
            check_linux_rust
            ;;
        security)
            check_repository_contract
            check_security_tools
            ;;
        architecture)
            check_repository_contract
            ;;
        release)
            check_repository_contract
            if [[ "$(uname -s)" == "Linux" && "$(uname -m)" == "x86_64" ]]; then
                check_linux_rust
            else
                check_portable_rust
            fi
            "${SCRIPT_DIR}/package-release.sh"
            ;;
        kvm)
            "${SCRIPT_DIR}/kvm-smoke.sh"
            ;;
        -h|--help|help)
            usage
            ;;
        *)
            printf 'unknown check profile: %s\n' "$profile" >&2
            usage >&2
            return 2
            ;;
    esac
}

main "$@"
