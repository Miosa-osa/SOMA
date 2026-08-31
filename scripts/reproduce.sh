#!/usr/bin/env bash
#
# One command from an OCI image to a measured sandbox launch.
#
# Producing a single benchmark number used to take seven commands in a fixed order, three of
# which fail silently when skipped: a Generation with no captured snapshot cold boots and looks
# like a working measurement, a shape that differs from the capture shape is refused before a
# machine exists, and a store built against an older wire contract cannot launch at all. This
# script performs every step, checks every precondition first, and refuses loudly rather than
# producing a number that means something other than what it says.
#
# Usage:
#   scripts/reproduce.sh [options] <image> [-- command ...]
#
# Example:
#   scripts/reproduce.sh --memory-mib 1024 --samples 5 node:22 -- /usr/local/bin/node --version

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
cd "$REPO_ROOT"

readonly MARKER="soma-reproduce-ok"
readonly MUSL_TARGET="x86_64-unknown-linux-musl"
readonly AGENT_BIN="target/${MUSL_TARGET}/release/soma-guest-agent"
readonly STAMP_NAME=".soma-reproduce-stamp"

IMAGE=""
MEM_MIB=1024
DISK_MIB=10240
VCPUS=1
SAMPLES=5
CONCURRENCY=1
ROOT_DIR="${SOMA_REPRODUCE_ROOT:-/var/tmp/soma-reproduce}"
FS_TOOLS="${SOMA_FS_TOOLS:-/srv/soma/fs-tools}"
COMMAND=(/bin/sh -c "echo ${MARKER}")
EXPECT="$MARKER"
CUSTOM_COMMAND=0
STORE_DIR=""
ENTRY=""
KERNEL=""

log() { printf '\n==> %s\n' "$1"; }
die() { printf '\nreproduce: %s\n' "$1" >&2; exit 1; }

usage() {
    cat <<'EOF'
usage: scripts/reproduce.sh [options] <image> [-- command ...]

  --memory-mib N    guest memory; part of the shape a snapshot is captured at (default 1024)
  --storage-mib N   guest storage; part of the shape (default 10240)
  --vcpus N         guest vCPUs; part of the shape (default 1)
  --samples N       launches to measure (default 5)
  --concurrency N   launches to start together per round (default 1)
  --root DIR        scratch root for prepared stores and heads (default /var/tmp/soma-reproduce)
  --fs-tools DIR    pinned filesystem tools (default /srv/soma/fs-tools)
  --expect TEXT     text the guest command must print (default the built in marker)
  -h, --help        this text

The command after -- runs inside the sandbox. Its default prints a marker this script checks,
so any image with /bin/sh measures without naming a program.
EOF
}

parse_args() {
    while (( $# > 0 )); do
        case "$1" in
            --memory-mib) MEM_MIB="${2:?--memory-mib needs a value}"; shift 2 ;;
            --storage-mib) DISK_MIB="${2:?--storage-mib needs a value}"; shift 2 ;;
            --vcpus) VCPUS="${2:?--vcpus needs a value}"; shift 2 ;;
            --samples) SAMPLES="${2:?--samples needs a value}"; shift 2 ;;
            --concurrency) CONCURRENCY="${2:?--concurrency needs a value}"; shift 2 ;;
            --root) ROOT_DIR="${2:?--root needs a value}"; shift 2 ;;
            --fs-tools) FS_TOOLS="${2:?--fs-tools needs a value}"; shift 2 ;;
            --expect) EXPECT="${2:?--expect needs a value}"; shift 2 ;;
            -h|--help) usage; exit 0 ;;
            --) shift; COMMAND=("$@"); CUSTOM_COMMAND=1; break ;;
            -*) usage >&2; die "unknown option $1" ;;
            *) [[ -z "$IMAGE" ]] || die "only one image may be given, found $IMAGE and $1"
               IMAGE="$1"; shift ;;
        esac
    done
    [[ -n "$IMAGE" ]] || { usage >&2; die "no image given"; }
    if (( CUSTOM_COMMAND == 1 )); then
        (( ${#COMMAND[@]} > 0 )) || die "-- was given with no command after it"
        [[ "$EXPECT" != "$MARKER" ]] || \
            die "a command after -- needs --expect naming text its output must contain"
    fi
}

# The versions that decide whether a prepared store can still be launched. A store built before
# any of these changed cannot restore, and the mismatch is what this script reports rather than
# letting the launch fail with a message about bytes.
contract_fingerprint() {
    local pairs=(
        "crates/soma-generation/src/generation/contracts.rs:_VERSION: u16 ="
        "crates/soma-generation/src/generation/initramfs.rs:INITRAMFS_LAYOUT_VERSION"
        "crates/soma-generation/src/generation/manifest.rs:MANIFEST_SCHEMA_VERSION"
        "crates/soma-kvm/src/snapshot/manifest.rs:SCHEMA_VERSION"
        "crates/soma-guest/src/launch_page/wire.rs:PAGE_SCHEMA_VERSION"
    )
    local pair file pattern found
    for pair in "${pairs[@]}"; do
        file="${pair%%:*}"
        pattern="${pair#*:}"
        [[ -f "$file" ]] || die "the wire contract file $file is missing; the checkout is not SOMA"
        found="$(grep -F "$pattern" "$file" | grep -F 'const' || true)"
        [[ -n "$found" ]] || die "no $pattern in $file; update contract_fingerprint in this script"
        printf '%s %s\n' "$file" "$found"
    done | tr -s '[:space:]' ' ' | sha256sum | cut -c1-16
}

preflight() {
    log "preflight"
    [[ "$(uname -s)" == "Linux" && "$(uname -m)" == "x86_64" ]] || \
        die "the KVM engine runs on Linux x86_64 only, found $(uname -s) $(uname -m)"
    [[ -r /dev/kvm && -w /dev/kvm ]] || \
        die "/dev/kvm is not readable and writable by this account; scripts/setup-host.sh fixes that"

    if ! command -v cargo >/dev/null 2>&1 && [[ -f "$HOME/.cargo/env" ]]; then
        # cargo is on the interactive PATH only, which is why non-interactive runs fail here.
        # shellcheck source=/dev/null
        . "$HOME/.cargo/env"
    fi
    local tool
    for tool in cargo skopeo python3 sha256sum; do
        command -v "$tool" >/dev/null 2>&1 || die "$tool is not on PATH; scripts/setup-host.sh installs it"
    done

    # kernel/out is a symlink to a built kernel tree. On a host where that link dangles every
    # prepare fails deep inside skopeo output, so name it here instead.
    if [[ -L kernel/out && ! -d kernel/out ]]; then
        die "kernel/out is a dangling symlink to $(readlink kernel/out); build it with kernel/build.sh or repoint the link"
    fi
    [[ -e kernel/out ]] || die "kernel/out does not exist; run scripts/build-soma.sh"
    KERNEL="$(compgen -G 'kernel/out/vmlinux-*-soma-v1' | head -1 || true)"
    [[ -n "$KERNEL" ]] || die "kernel/out holds no vmlinux-*-soma-v1; run scripts/build-soma.sh"

    [[ -d "$FS_TOOLS/erofs" && -d "$FS_TOOLS/e2fsprogs" ]] || \
        die "$FS_TOOLS has no erofs/ and e2fsprogs/; run scripts/build-fs-tools.sh or pass --fs-tools"

    local positive
    for positive in MEM_MIB DISK_MIB VCPUS SAMPLES CONCURRENCY; do
        [[ "${!positive}" =~ ^[1-9][0-9]*$ ]] || die "--${positive,,} must be a positive integer, found ${!positive}"
    done
    printf '  host ok, kernel %s\n' "$KERNEL"
}

build() {
    log "building the workspace, the guest agent, and the two tools"
    cargo build --release --workspace
    [[ -f "$AGENT_BIN" ]] || ./scripts/build-guest-agent.sh >/dev/null
    [[ -f "$AGENT_BIN" ]] || die "the static guest agent did not build; see scripts/build-guest-agent.sh"
    cargo build --release -p soma-generation --example prepare_generation
    cargo build --release -p soma-local --example capture_snapshot
}

# One store per image and shape, because a snapshot restores exactly the memory it was captured
# with. Sharing a store across shapes is the mistake that reports zero successes and no reason.
store_path() {
    local slug
    slug="$(printf '%s' "$IMAGE" | tr -c 'A-Za-z0-9._-' '-')"
    printf '%s/store/%s-%s-%s-%s\n' "$ROOT_DIR" "$slug" "$VCPUS" "$MEM_MIB" "$DISK_MIB"
}

write_stamp() {
    cat >"$STORE_DIR/$STAMP_NAME" <<EOF
image=$IMAGE
vcpus=$VCPUS
memory_mib=$MEM_MIB
storage_mib=$DISK_MIB
contract=$(contract_fingerprint)
EOF
}

# Refuses a store that cannot answer the launch that is about to be made. Every branch names the
# store, what it holds, what was asked for, and the one command that resolves it.
verify_store() {
    log "verifying the prepared store"
    local stamp="$STORE_DIR/$STAMP_NAME" want
    want="$(contract_fingerprint)"
    [[ -f "$stamp" ]] || die "$STORE_DIR has no $STAMP_NAME, so what it was built against is unknown; treat it as stale and delete it, then rerun"

    local key expected
    for key in image vcpus memory_mib storage_mib; do
        case "$key" in
            image) expected="$IMAGE" ;; vcpus) expected="$VCPUS" ;;
            memory_mib) expected="$MEM_MIB" ;; *) expected="$DISK_MIB" ;;
        esac
        local have
        have="$(grep "^$key=" "$stamp" | cut -d= -f2- || true)"
        [[ "$have" == "$expected" ]] || die "$STORE_DIR was captured at $key=$have but this run asks for $expected; a restore must match the capture shape exactly, so prepare a separate store for it"
    done

    local have_contract
    have_contract="$(grep '^contract=' "$stamp" | cut -d= -f2- || true)"
    [[ "$have_contract" == "$want" ]] || die "$STORE_DIR was built against wire contract $have_contract and this checkout is $want; the store is stale and cannot launch, delete it and rerun"

    ENTRY="$(compgen -G "$STORE_DIR/ref-*" | head -1 || true)"
    [[ -n "$ENTRY" ]] || die "$STORE_DIR holds no ref-* entry; delete the store and rerun to prepare it"
    [[ -f "$ENTRY/candidate.somacan" ]] || die "$ENTRY has no candidate.somacan; the prepare step did not finish, delete the store and rerun"

    # A prepared entry with no snapshot does not refuse. It cold boots, about fifteen times
    # slower, and reads exactly like a working measurement. That is the silent failure.
    [[ -d "$ENTRY/snapshot" && -f "$ENTRY/snapshot/state.somasnap" ]] || \
        die "$ENTRY has no captured snapshot, so every launch would cold boot and report a number about fifteen times slower with no error; capture it with target/release/examples/capture_snapshot $ENTRY $MEM_MIB"
    printf '  entry %s\n  shape %s vCPU, %s MiB memory, %s MiB storage, contract %s\n' \
        "$ENTRY" "$VCPUS" "$MEM_MIB" "$DISK_MIB" "$want"
}

prepare() {
    STORE_DIR="$(store_path)"
    if [[ -f "$STORE_DIR/$STAMP_NAME" ]]; then
        log "reusing the prepared store at $STORE_DIR"
        return
    fi
    log "compiling $IMAGE into a Generation at $STORE_DIR"
    rm -rf -- "$STORE_DIR"
    mkdir -p "$STORE_DIR"
    ./scripts/prepare-generation.sh "$IMAGE" "$STORE_DIR" "$FS_TOOLS" "$MEM_MIB" "$DISK_MIB"
    local entry
    entry="$(compgen -G "$STORE_DIR/ref-*" | head -1 || true)"
    [[ -n "$entry" ]] || die "prepare-generation.sh wrote no ref-* entry into $STORE_DIR"

    # prepare-generation.sh compiles and does not capture. Capturing here is what stops the
    # cold boot that would otherwise be reported as a result.
    log "capturing the snapshot at ${MEM_MIB} MiB"
    ./target/release/examples/capture_snapshot "$entry" "$MEM_MIB"
    write_stamp
}

measure() {
    log "measuring $SAMPLES launches at concurrency $CONCURRENCY"
    local raw
    raw="$(mktemp -d "$ROOT_DIR/.measure.XXXXXXXX")"
    export SOMA_GENERATION_STORE="$STORE_DIR"
    export SOMA_HEAD_DIR="$ROOT_DIR/heads"
    export SOMA_ALLOW_UNCERTIFIED_GENERATION=1
    mkdir -p "$SOMA_HEAD_DIR"

    launch() {
        ./target/release/soma --format json --backend kvm run \
            --vcpus "$VCPUS" --memory-mib "$MEM_MIB" --storage-mib "$DISK_MIB" \
            "$IMAGE" -- "${COMMAND[@]}" >"$raw/$1.json" 2>"$raw/$1.err" || true
    }

    # One warming launch, discarded, so the first measured sample does not pay the first touch
    # of the page cache and stand in for the steady state.
    launch warm
    rm -f "$raw/warm.json" "$raw/warm.err"

    local index=0 slot
    while (( index < SAMPLES )); do
        for (( slot = 0; slot < CONCURRENCY && index < SAMPLES; slot++ )); do
            launch "$index" &
            index=$(( index + 1 ))
        done
        wait
    done

    local status=0
    python3 scripts/reproduce-report.py "$raw" "$EXPECT" "$IMAGE" "$VCPUS" "$MEM_MIB" \
        "$CONCURRENCY" || status=$?
    if (( status != 0 )); then
        printf '\nreproduce: the first failing launch reported:\n' >&2
        head -20 "$raw"/*.err 2>/dev/null >&2 || true
    fi
    rm -rf -- "$raw"
    return "$status"
}

main() {
    parse_args "$@"
    preflight
    mkdir -p "$ROOT_DIR"
    build
    prepare
    verify_store
    measure
    printf '\n  the store is kept at %s so a rerun measures instead of rebuilding.\n' "$STORE_DIR"
    printf '  remove it with: rm -rf %s\n' "$STORE_DIR"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
