#!/usr/bin/env bash
# Runs the soma-netd live Linux tests inside the pinned privileged Ubuntu 24.04 container.
#
# The test binary is built on the host with the workspace toolchain and bind-mounted into the
# container, which provides CAP_NET_ADMIN, /dev/net/tun, iproute2, nftables, and conntrack.
# The container image is built from scripts/netd-live/Dockerfile, whose base is pinned by
# content digest.  Output is written to stdout and, when SOMA_NETD_LIVE_LOG is set, to that file.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIR
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd -P)"
readonly REPO_ROOT
readonly IMAGE="soma-netd-live:local"

cd "${REPO_ROOT}"

if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
    printf 'soma-netd live tests require a Linux x86_64 host with Docker\n' >&2
    exit 1
fi

for tool in cargo docker python3; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf 'required command not found: %s\n' "$tool" >&2
        exit 1
    fi
done

printf '==> building live-test image %s\n' "${IMAGE}"
docker build -q -t "${IMAGE}" "${SCRIPT_DIR}/netd-live" >/dev/null
IMAGE_ID="$(docker image inspect --format '{{.Id}}' "${IMAGE}")"
BASE_DIGEST="$(grep -E '^FROM ' "${SCRIPT_DIR}/netd-live/Dockerfile" | head -n 1 | sed 's/^FROM //')"

printf '==> building soma-netd test binary\n'
TEST_BINARY="$(
    cargo test --locked -p soma-netd --test live_linux --no-run --message-format=json 2>/dev/null \
        | python3 -c 'import json,sys
for line in sys.stdin:
    try:
        message = json.loads(line)
    except ValueError:
        continue
    if message.get("reason") == "compiler-artifact" and message.get("executable") and message["target"]["name"] == "live_linux":
        print(message["executable"])'
)"
if [[ -z "${TEST_BINARY}" ]]; then
    printf 'could not locate the live_linux test executable\n' >&2
    exit 1
fi

printf '==> host kernel: %s\n' "$(uname -r)"
printf '==> image: %s (base %s)\n' "${IMAGE_ID}" "${BASE_DIGEST}"
printf '==> git revision: %s\n' "$(git rev-parse --short HEAD 2>/dev/null || printf unknown)"

run_container() {
    docker run --rm --privileged \
        --mount "type=bind,source=${TEST_BINARY},target=/work/live_linux,readonly" \
        "${IMAGE}" \
        sh -c '
            printf "container kernel: %s\n" "$(uname -r)"
            nft --version; ip -V; conntrack --version
            /work/live_linux --ignored --test-threads=1 --nocapture
            status=$?
            printf "==> post-run namespaces (expect none):\n"; ip netns list
            printf "==> post-run links (expect only lo and the container uplink):\n"; ip -brief link
            printf "==> post-run nft tables (expect none):\n"; nft list tables
            exit "$status"'
}

if [[ -n "${SOMA_NETD_LIVE_LOG:-}" ]]; then
    run_container 2>&1 | tee "${SOMA_NETD_LIVE_LOG}"
else
    run_container
fi
