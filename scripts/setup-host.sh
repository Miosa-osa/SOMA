#!/usr/bin/env bash
#
# Prepares a fresh Ubuntu x86_64 host to build and run the SOMA engine.
#
# It installs every host dependency, adds the current user to the kvm and docker groups, installs
# the pinned Rust toolchain and the musl target the guest agent needs, and prints a final report.
# It is idempotent: run it again and it only fills gaps.
#
# It does not build SOMA.
# Run it from an obtained SOMA repository so its pinned toolchain file is available.
#
# Requires sudo for package installation. The kernel is built inside a container, so no kernel
# build dependencies are installed on the host.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
readonly MUSL_TARGET="x86_64-unknown-linux-musl"
readonly SOMA_WORK_ROOT="${SOMA_WORK_ROOT:-/srv/soma}"
REQUIRED_FAILURES=0

log() { printf '\n==> %s\n' "$1"; }
warn() { printf '  WARN  %s\n' "$1" >&2; }

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
        util-linux
    )
    sudo apt-get update -qq
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "${packages[@]}"
}

enable_services() {
    log "enabling the container builder"
    sudo systemctl enable --now docker.service >/dev/null
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

provision_work_root() {
    log "provisioning $SOMA_WORK_ROOT"
    local user="${SUDO_USER:-$USER}"
    local group
    group="$(id -gn "$user")"
    sudo install -d -m 0750 -o "$user" -g "$group" \
        "$SOMA_WORK_ROOT" \
        "$SOMA_WORK_ROOT/fs-tools" \
        "$SOMA_WORK_ROOT/prepared" \
        "$SOMA_WORK_ROOT/heads"
}

install_rust() {
    log "installing the Rust toolchain"
    if ! command -v rustup >/dev/null 2>&1; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
        # shellcheck disable=SC1091
        source "${CARGO_HOME:-$HOME/.cargo}/env"
    fi
    cd "$REPO_ROOT"
    # rust-toolchain.toml is the one source of truth for the exact toolchain.
    rustup show >/dev/null
    rustup target add "$MUSL_TARGET" >/dev/null
}

report() {
    log "host readiness"
    required() {
        local label="$1"
        shift
        if "$@" >/dev/null 2>&1; then
            printf '  ok    %s\n' "$label"
        else
            printf '  FAIL  %s\n' "$label"
            REQUIRED_FAILURES=$((REQUIRED_FAILURES + 1))
        fi
    }
    required "KVM character device present" test -c /dev/kvm
    required "CPU virtualization exposed" grep -Eq 'vmx|svm' /proc/cpuinfo
    required "cgroup v2 mounted" test -f /sys/fs/cgroup/cgroup.controllers
    required "seccomp actions exposed" test -r /proc/sys/kernel/seccomp/actions_avail
    required "Docker daemon reachable as root" sudo docker info
    required "Python 3 present" command -v python3
    required "Cargo present" command -v cargo
    required "musl target installed" bash -c \
        "rustup target list --installed 2>/dev/null | grep -qx '$MUSL_TARGET'"
    required "SOMA work root writable by the operator" test -w "$SOMA_WORK_ROOT"

    if findmnt -rno FSTYPE | grep -qx xfs; then
        printf '  ok    XFS filesystem present for later reflink validation\n'
    else
        warn "no XFS filesystem is mounted; the development path can copy disks, but prepared performance cannot be evaluated"
    fi

    if [[ "$REQUIRED_FAILURES" -eq 0 ]]; then
        printf '\nHost is ready. Open a new shell for group changes, then build SOMA.\n'
    else
        printf '\nHost is not fully ready. Address the FAIL and WARN lines above.\n'
        return 1
    fi
}

main() {
    require_linux_x86_64
    apt_packages
    enable_services
    enable_groups
    provision_work_root
    install_rust
    report
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
