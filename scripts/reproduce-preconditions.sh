#!/usr/bin/env bash
#
# Whether this run can produce a number that means what it says.
#
# Sourced by reproduce.sh, which owns the steps; this file owns the refusals. Every check here
# exists because skipping it produces a plausible wrong answer rather than an error: a store with
# no captured snapshot cold boots and reads about fifteen times slower, a shape that differs from
# the capture shape is refused before a machine exists, and a store built against an older wire
# contract cannot launch at all.
#
# Each function uses the log() and die() defined by the caller, and reads the caller's shape
# variables. It is not executable on its own.

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
