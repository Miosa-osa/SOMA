#!/usr/bin/env bash
#
# Repeats the whole desktop proof (milestone M1 of the cua desktop design) with one command.
#
# Given a desktop Generation compiled by scripts/desktop-generation.sh, it launches one desktop
# sandbox through soma-api, starts its session through exec, drives it with cua-driver through exec
# the way an agent would, measures, destroys it and checks that nothing was left behind. soma-netd and soma-api run in a privileged container
# with their own network namespace (scripts/desktop-proof/runtime.Dockerfile), so the host's own
# network configuration is not changed; the container is removed at the end.
#
# Run it as root from the repository root on the KVM host, after build-soma.sh and
# `cargo build --release -p soma-api -p soma-netd`.
#
# Usage:
#   scripts/desktop-proof.sh --store DIR --reference REF --work DIR [--hold SECONDS] [--keep]
#
#   --hold SECONDS  keep the desktop up that long (or until <work>/release exists)
#   --keep          leave the runtime container running afterwards, for inspection
#
# Everything the proof saw is written under <work>/results: every API request and response, the
# screenshots, the timings (measurements.tsv) and the residue check (residue.txt).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
readonly CONTAINER=soma-desktop-proof
readonly RUNTIME_IMAGE=soma-desktop-proof-runtime:1

STORE="" REFERENCE="" WORK="" HOLD=0 KEEP=0
while (( $# > 0 )); do
    case "$1" in
        --store) STORE="$2"; shift 2 ;;
        --reference) REFERENCE="$2"; shift 2 ;;
        --work) WORK="$2"; shift 2 ;;
        --hold) HOLD="$2"; shift 2 ;;
        --keep) KEEP=1; shift ;;
        *) printf 'unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done

die() { printf 'desktop-proof: %s\n' "$1" >&2; exit 1; }
[[ -n "$STORE" && -n "$REFERENCE" && -n "$WORK" ]] || die "--store, --reference and --work are required"
[[ "$(id -u)" == 0 ]] || die "run as root: the runtime needs /dev/kvm and a privileged container"
for binary in soma-api soma-netd; do
    [[ -x "$REPO_ROOT/target/release/$binary" ]] || die "target/release/$binary is missing; build it first"
done
ENTRY="$STORE/ref-$(printf '%s' "$REFERENCE" | sha256sum | cut -d' ' -f1)"
[[ -s "$ENTRY/generation.id" ]] || die "$REFERENCE has no certified Generation in $STORE; run scripts/desktop-generation.sh"
mkdir -p "$WORK"
WORK="$(cd "$WORK" && pwd -P)"
STORE="$(cd "$STORE" && pwd -P)"
[[ ! -e "$WORK/results" ]] || die "$WORK/results already exists; use a new --work directory"

# The host's network namespaces, nftables tables and links. Docker's own per-container veth links
# come and go with any container on the host, so they are left out; soma-netd's links are made in
# the container's namespace and can never appear on the host's list in the first place.
host_network() {
    { ip netns list; nft list tables; ip -br link | awk '{print $1}' | grep -v '^veth'; } 2>/dev/null | sort
}

cleanup() {
    if (( ! KEEP )); then
        docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

host_network >"$WORK/host-network.before"
docker build -q -t "$RUNTIME_IMAGE" -f "$REPO_ROOT/scripts/desktop-proof/runtime.Dockerfile" \
    "$REPO_ROOT/scripts/desktop-proof" >/dev/null
docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
docker run -d --name "$CONTAINER" --privileged --cgroupns=private --tmpfs /run \
    -v "$REPO_ROOT:$REPO_ROOT:ro" -v "$STORE:$STORE" -v "$WORK:$WORK" \
    "$RUNTIME_IMAGE" sleep infinity >/dev/null

status=0
docker exec "$CONTAINER" bash "$REPO_ROOT/scripts/desktop-proof/run.sh" \
    "$REPO_ROOT" "$STORE" "$REFERENCE" "$WORK" "$HOLD" || status=$?

cleanup
if (( ! KEEP )); then
    # The container's namespace is gone with it; what is left to check is the host's own network:
    # the only difference allowed is none at all.
    host_network >"$WORK/host-network.after"
    if diff "$WORK/host-network.before" "$WORK/host-network.after" >"$WORK/host-network.diff"; then
        printf 'host network: unchanged\n' | tee -a "$WORK/results/residue.txt"
    else
        printf 'host network: CHANGED\n' | tee -a "$WORK/results/residue.txt"
        cat "$WORK/host-network.diff"
        status=1
    fi
fi
exit "$status"
