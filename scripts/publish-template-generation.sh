#!/usr/bin/env bash
# Compile a locally supplied, digest-pinned OCI template and install its certified Generation.
# The caller supplies the verified host release bundle and operator-owned build inputs.
set -euo pipefail
umask 077

fail() { printf 'publish-template-generation: %s\n' "$1" >&2; exit 1; }

if (( $# != 10 )); then
    printf 'usage: %s <release-dir> <prepared-root> <pinned-reference> <memory-mib> <template.toml> <oci-layout> <kernel> <kernel-config> <erofs-tools> <e2fsprogs>\n' "${0##*/}" >&2
    exit 2
fi

release=$(realpath -e -- "$1")
root=$2
reference=$3
memory_mib=$4
template=$(realpath -e -- "$5")
layout=$(realpath -e -- "$6")
kernel=$(realpath -e -- "$7")
kernel_config=$(realpath -e -- "$8")
erofs_tools=$(realpath -e -- "$9")
e2fsprogs=$(realpath -e -- "${10}")

[[ "$reference" =~ ^[a-z0-9][a-z0-9._:/-]*@sha256:[a-f0-9]{64}$ ]] ||
    fail "the reference must be pinned to an OCI digest"
[[ "$memory_mib" =~ ^[1-9][0-9]*$ ]] || fail "memory must be a positive integer in MiB"
[[ -d "$root" && ! -L "$root" && -O "$root" ]] ||
    fail "the prepared root must be a directory owned by the publisher"
root=$(realpath -e -- "$root")
root_mode=$(stat -c %a -- "$root")
(( (8#$root_mode & 022) == 0 )) || fail "the prepared root must not be writable by group or others"

incoming="$root/.incoming"
if [[ ! -e "$incoming" ]]; then
    mkdir -m 0700 -- "$incoming" 2>/dev/null || [[ -d "$incoming" ]]
fi
[[ -d "$incoming" && ! -L "$incoming" && -O "$incoming" ]] ||
    fail "the incoming directory must be owned by the publisher"
incoming_mode=$(stat -c %a -- "$incoming")
(( (8#$incoming_mode & 077) == 0 )) || fail "the incoming directory must be private"

reservation=$(mktemp -d "$incoming/template.XXXXXXXX")
stage="${reservation##*/}-entry"
entry="$incoming/$stage"
cleanup() {
    rm -rf -- "$entry"
    rmdir -- "$reservation"
}
trap cleanup EXIT

"$release/prepare_from_template" "$template" "$layout" "$kernel" "$kernel_config" \
    "$release/soma-guest-agent" "$erofs_tools" "$e2fsprogs" "$entry" >&2
cmp -s "$entry/reference" <(printf '%s' "$reference") ||
    fail "the compiled template reference differs from the approved OCI reference"

"$release/capture_snapshot" "$entry" "$memory_mib" >&2
generation=$(cat -- "$entry/generation.id")
[[ "$generation" =~ ^sha256:[a-f0-9]{64}$ ]] || fail "snapshot capture produced no certified Generation"
"$release/install_generation" "$root" "$stage" "$reference" "$generation" >&2
printf '%s\n' "$generation"
