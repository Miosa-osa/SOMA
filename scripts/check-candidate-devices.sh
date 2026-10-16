#!/usr/bin/env bash
#
# Refuses to let a new sized Generation entry be installed unless its machine has the same device list as an
# entry that is already serving: same virtio_mmio devices on the kernel command line, soma.net=eth0 present,
# same snapshot state size class. A Generation prepared without SOMA_PREPARE_NETWORK=public_internet has no
# network device and soma-api rejects it with an opaque workload_rejected.
#
# Usage: check-candidate-devices.sh <new-entry-dir> <serving-entry-dir>
set -euo pipefail
NEW="${1:?new entry dir}"
OLD="${2:?serving entry dir}"

cmdline() {
    tr -c '[:print:]' '\n' <"$1/candidate.somacan" | grep -o 'console=ttyS0.*' | head -1
}
devices() { printf '%s\n' "$1" | tr ' ' '\n' | grep -E '^(virtio_mmio.device=|soma\.net=)' | sort; }

new_line="$(cmdline "${NEW}")"
old_line="$(cmdline "${OLD}")"
[[ -n "${new_line}" && -n "${old_line}" ]] || { echo "no kernel command line in a candidate" >&2; exit 1; }

if ! diff <(devices "${old_line}") <(devices "${new_line}") >/dev/null; then
    echo "DEVICE LIST DIFFERS from the serving entry:" >&2
    diff <(devices "${old_line}") <(devices "${new_line}") >&2 || true
    exit 1
fi
grep -q 'soma.net=eth0' <<<"${new_line}" || { echo "no network device (soma.net=eth0) in the new entry" >&2; exit 1; }

old_state="$(stat -c %s "${OLD}/snapshot/state.somasnap")"
new_state="$(stat -c %s "${NEW}/snapshot/state.somasnap")"
# The state file holds the device and vCPU state: a very different size means a different machine shape.
if (( new_state * 2 < old_state || new_state > old_state * 2 )); then
    echo "snapshot state size ${new_state} vs serving ${old_state}: different machine shape" >&2
    exit 1
fi
echo "ok: device list and state size match the serving entry"
