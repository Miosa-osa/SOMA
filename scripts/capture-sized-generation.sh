#!/usr/bin/env bash
#
# Prepares and captures one sized Generation (M05, M10, G10, G20 or G40) of a sandbox image for one CPU class.
#
# Usage (run as root on a host of the target CPU class, from the directory that holds the inputs):
#   capture-sized-generation.sh <M05|M10|G10|G20|G40> <reference-suffix> <work-dir>
#
# M05 and M10 boot at 512 and 1024 MiB: soma-api refuses a shape below an entry's boot memory, so the
# micro (1x512) and tiny (1x1024) sizes need their own entries; G10, G20 and G40 boot at 2048 MiB.
#
# <work-dir> holds: fs-tools/ (or set FS_TOOLS), in/{prepare_generation,capture_snapshot,vmlinux-soma-v1,config-x86_64-soma-v1,soma-guest-agent.pinned}
# and oci/ (the image as an OCI layout). The entry is written to <work-dir>/store/ref-<sha256 of the reference>.
#
# The flags below are what the customer-facing sized Generations carry. Leaving SOMA_PREPARE_NETWORK or
# SOMA_PREPARE_VCPUS out builds an entry with no network device or the wrong vCPU count, which soma-api answers
# with workload_rejected and no detail (found the hard way on 2026-10-12), so they are set here and checked by
# check-candidate-devices.sh before an entry is installed.
set -euo pipefail

NAME="${1:?M05|M10|G10|G20|G40}"
SUFFIX="${2:?reference suffix, e.g. b2 (a new image needs a new reference: the reference is bound into the entry)}"
W="$(cd "${3:?work dir}" && pwd -P)"
FS_TOOLS="${FS_TOOLS:-${W}/fs-tools}"   # the pinned erofs and e2fsprogs tools (build-fs-tools.sh)

BOOT_MIB=2048
case "${NAME}" in
    M05) DISK=10240 BOOT_MIB=512 ;;
    M10) DISK=20480 BOOT_MIB=1024 ;;
    G10) DISK=10240 ;;
    G20) DISK=20480 ;;
    G40) DISK=40960 ;;
    *) echo "unknown class ${NAME}" >&2; exit 2 ;;
esac

REF="soma-default:${NAME}-${SUFFIX}"
ENTRY="${W}/store/ref-$(printf '%s' "${REF}" | sha256sum | cut -d' ' -f1)"
mkdir -p "${W}/store"
rm -r --one-file-system "${ENTRY}" 2>/dev/null || true

echo "start $(date -u +%T) ${REF} entry=${ENTRY}"
SOMA_PREPARE_VCPUS=8 SOMA_PREPARE_NETWORK=public_internet SOMA_PREPARE_CEILING_MIB=16384 \
    ionice -c3 nice -n19 "${W}/in/prepare_generation" "${REF}" "${W}/oci" \
    "${W}/in/vmlinux-soma-v1" "${W}/in/config-x86_64-soma-v1" "${W}/in/soma-guest-agent.pinned" \
    "${FS_TOOLS}/erofs" "${FS_TOOLS}/e2fsprogs" "${ENTRY}" "${BOOT_MIB}" "${DISK}"
echo "prepared $(date -u +%T)"
ionice -c3 nice -n19 "${W}/in/capture_snapshot" "${ENTRY}" "${BOOT_MIB}" 2>&1 | tail -5 | cut -c1-200
echo "captured $(date -u +%T)"
cat "${ENTRY}/generation.id"
echo
