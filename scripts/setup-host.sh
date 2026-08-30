#!/usr/bin/env bash
#
# Prepares a fresh Ubuntu x86_64 host to build and run the SOMA engine.
#
# It installs every host dependency, adds the current user to the kvm and docker groups, installs
# the pinned Rust toolchain and the musl target the guest agent needs, and prints a final report.
# It is idempotent: run it again and it only fills gaps.
#
# It does not build SOMA, and it does not need SOMA to be cloned yet. It sets up the host so the
# rest of the server-setup runbook can run.
#
# Requires sudo for package installation. The kernel is built inside a container, so no kernel
# build dependencies are installed on the host.

set -euo pipefail

readonly MUSL_TARGET="x86_64-unknown-linux-musl"
FAIL=0

log() { printf '\n==> %s\n' "$1"; }
warn() { printf '  WARN: %s\n' "$1" >&2; FAIL=1; }

require_linux_x86_64() {
    if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
        printf 'SOMA engine hosts are Linux x86_64; found %s %s\n' "$(uname -s)" "$(uname -m)" >&2
        exit 1
    fi
}

apt_packages() {
    log "installing host packages"
    # ca-certificates and curl are needed to fetch rustup over https; python3 runs the kernel
    # verify scripts and parts of the harness; the rest are the runtime and storage tools. The
    # host build itself is pure Rust, so no openssl or kernel build packages are installed here.
    local packages=(
        ca-certificates curl git python3 tar xz-utils
        build-essential
        docker.io
        xfsprogs e2fsprogs erofs-utils
        skopeo
    )
    sudo apt-get update -qq
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "${packages[@]}"
}

enable_groups() {
    log "granting the current user kvm and docker access"
    local user="${SUDO_USER:-$USER}"
    for group in kvm docker; do
        if getent group "$group" >/dev/null; then
            sudo usermod -aG "$group" "$user"
        else
            warn "group $group does not exist; is KVM or docker installed?"
        fi
    done
    printf '  note: log out and back in for new group membership to take effect\n'
}

install_rust() {
    log "installing the Rust toolchain"
    if ! command -v rustup >/dev/null 2>&1; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
        # shellcheck disable=SC1091
        source "${CARGO_HOME:-$HOME/.cargo}/env"
    fi
    # rust-toolchain.toml pins the channel; installing it here means the first cargo build does
    # not stop to download a toolchain. Fall back to a known-good channel if run outside the repo.
    if [[ -f rust-toolchain.toml ]]; then
        rustup show >/dev/null
    else
        rustup toolchain install 1.98.0 >/dev/null
    fi
    rustup target add "$MUSL_TARGET" >/dev/null
}

report() {
    log "host readiness"
    local ok=1
    check() {
        if eval "$2" >/dev/null 2>&1; then
            printf '  ok    %s\n' "$1"
        else
            printf '  FAIL  %s\n' "$1"
            ok=0
        fi
    }
    check "kvm device present" 'test -e /dev/kvm'
    check "cpu virtualization" 'grep -Eq "vmx|svm" /proc/cpuinfo'
    check "docker present" 'command -v docker'
    check "python3 present" 'command -v python3'
    check "cargo present" "command -v cargo"
    check "musl target installed" "rustup target list --installed 2>/dev/null | grep -qx $MUSL_TARGET"
    check "reflink filesystem somewhere" "findmnt -rno FSTYPE | grep -qx xfs"

    if [[ "$ok" -eq 1 && "$FAIL" -eq 0 ]]; then
        printf '\nHost is ready. Open a new shell for group changes, then build SOMA.\n'
    else
        printf '\nHost is not fully ready. Address the FAIL and WARN lines above.\n'
        printf 'A missing reflink filesystem is a performance warning, not a blocker.\n'
    fi
}

main() {
    require_linux_x86_64
    apt_packages
    enable_groups
    install_rust
    report
}

main "$@"
