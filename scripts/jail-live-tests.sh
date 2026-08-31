#!/usr/bin/env bash
#
# Builds the static jail probe, the static soma-vmm worker, and the jail_live test binary on
# the host, then runs the privileged live tests as root inside a pinned Ubuntu 24.04 container
# with its own cgroup namespace and a delegated cgroup2 subtree.
#
# The host only needs cargo with the x86_64-unknown-linux-musl target and a Docker daemon
# that allows --privileged; unprivileged user namespaces may be blocked on the host itself.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
readonly TARGET="x86_64-unknown-linux-musl"
readonly IMAGE="ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517"
readonly WORK="/work"

cd "${REPO_ROOT}"

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        printf 'required command not found: %s\n' "$1" >&2
        return 1
    fi
}

require_command cargo
require_command rustup
require_command docker
require_command python3
require_command file

if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
    printf 'jail live tests require a Linux x86_64 host, found %s %s\n' \
        "$(uname -s)" "$(uname -m)" >&2
    exit 1
fi

if ! rustup target list --installed | grep -qx "${TARGET}"; then
    printf 'missing Rust target %s; run: rustup target add %s\n' "${TARGET}" "${TARGET}" >&2
    exit 1
fi

printf '==> building the static jail-probe, soma-vmm, and jail_live binaries for %s\n' "${TARGET}"
cargo build --locked -p soma-jail --bin jail-probe --target "${TARGET}"
cargo build --locked -p soma-vmm --bin soma-vmm --target "${TARGET}"
test_binary="$(
    cargo test --locked -p soma-jail --test jail_live --target "${TARGET}" --no-run \
        --message-format=json \
        | python3 -c '
import json, sys
for line in sys.stdin:
    message = json.loads(line)
    if message.get("reason") != "compiler-artifact":
        continue
    if message["target"]["name"] == "jail_live" and message.get("executable"):
        print(message["executable"])
'
)"
probe_binary="${REPO_ROOT}/target/${TARGET}/debug/jail-probe"
vmm_binary="${REPO_ROOT}/target/${TARGET}/debug/soma-vmm"

for binary in "${test_binary}" "${probe_binary}" "${vmm_binary}"; do
    if [[ ! -x "${binary}" ]]; then
        printf 'expected binary is missing: %s\n' "${binary}" >&2
        exit 1
    fi
    description="$(file -b "${binary}")"
    if [[ "${description}" != *"static"*"linked"* ]]; then
        printf 'binary must be statically linked, found: %s\n' "${description}" >&2
        exit 1
    fi
done

relative_test="${test_binary#"${REPO_ROOT}"/}"
relative_probe="${probe_binary#"${REPO_ROOT}"/}"
relative_vmm="${vmm_binary#"${REPO_ROOT}"/}"

printf '==> image %s\n' "${IMAGE}"
docker image inspect --format '{{.Id}}' "${IMAGE}" >/dev/null 2>&1 || docker pull "${IMAGE}"

# Inside the container: move every process out of the cgroup namespace root so domain
# controllers can be delegated, create the delegated subtree, then run the ignored tests
# serially. The helper test is only ever spawned by the launcher-death test itself.
container_script="$(cat <<'INNER'
set -euo pipefail
printf 'kernel: %s\n' "$(uname -r)"
printf 'container identity: %s\n' "$(id)"
. /etc/os-release
printf 'image os: %s %s\n' "$ID" "$VERSION_ID"
mkdir -p /sys/fs/cgroup/init
while read -r pid; do
    echo "$pid" > /sys/fs/cgroup/init/cgroup.procs 2>/dev/null || true
done < /sys/fs/cgroup/cgroup.procs
echo '+cpu +memory +pids +io' > /sys/fs/cgroup/cgroup.subtree_control
mkdir /sys/fs/cgroup/soma-jail
echo '+cpu +memory +pids +io' > /sys/fs/cgroup/soma-jail/cgroup.subtree_control
printf 'delegated controllers: %s\n' "$(cat /sys/fs/cgroup/soma-jail/cgroup.subtree_control)"
ls -l /dev/kvm 2>/dev/null || printf '/dev/kvm: absent\n'
export SOMA_JAIL_CGROUP_ROOT=/sys/fs/cgroup/soma-jail
export SOMA_JAIL_ROOT_PARENT=/tmp/soma-jail-live
export SOMA_JAIL_PROBE="$WORK/$RELATIVE_PROBE"
export SOMA_VMM_BINARY="$WORK/$RELATIVE_VMM"
exec "$WORK/$RELATIVE_TEST" --ignored --test-threads=1 --skip helper_ "$@"
INNER
)"

printf '==> running jail_live inside the privileged container\n'
docker run --rm --privileged --cgroupns=private \
    --volume "${REPO_ROOT}:${WORK}:ro" \
    --env "WORK=${WORK}" \
    --env "RELATIVE_TEST=${relative_test}" \
    --env "RELATIVE_PROBE=${relative_probe}" \
    --env "RELATIVE_VMM=${relative_vmm}" \
    "${IMAGE}" bash -c "${container_script}" -- "$@"
