#!/usr/bin/env bash
#
# Proves the persistence gate decides pass from fail correctly, without a host.
#
# The gate's value is that it never reports green for a sandbox whose data did not come back, so
# the judgement it applies is worth holding still. This drives it with a fake adapter that can be
# told to lose the file or to leave a process running, and asserts that each defect turns the gate
# red for the right reason rather than being swallowed.

set -euo pipefail

TEST_REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
readonly TEST_REPO_ROOT
readonly GATE="$TEST_REPO_ROOT/scripts/persistence-gate.sh"

WORK="$(mktemp -d)"
readonly WORK
trap 'rm -rf -- "$WORK"' EXIT

cat >"$WORK/adapter" <<'ADAPTER'
#!/usr/bin/env bash
set -euo pipefail
state="${FAKE_STATE:?}"
action="$1"
case "$action" in
    write) printf '%s' "$4" >"$state/file" ;;
    read) cat "$state/file" 2>/dev/null || true ;;
    exec)
        if [[ "$3" == *pgrep* ]]; then
            [[ "${FAKE_SURVIVOR:-0}" == 1 ]] && printf '4242\n'
        else
            printf 'alive\n'
        fi
        ;;
    stop|start|reboot|api-restart) : ;;
    *) exit 2 ;;
esac
ADAPTER
chmod +x "$WORK/adapter"

run_gate() {
    local expect="$1" gate="$2" state="$3" survive="$4"
    shift 4
    local status=0
    FAKE_STATE="$state" FAKE_SURVIVOR="$survive" "$GATE" "$@" >"$WORK/out.txt" 2>&1 || status=$?
    if [[ "$expect" == pass && "$status" -ne 0 ]]; then
        printf 'gate %s should have passed but exited %s:\n' "$gate" "$status" >&2
        cat "$WORK/out.txt" >&2
        exit 1
    fi
    if [[ "$expect" != pass && "$status" -eq 0 ]]; then
        printf 'gate %s should have failed but passed\n' "$gate" >&2
        exit 1
    fi
}

mkdir -p "$WORK/good" "$WORK/survivor" "$WORK/lost"

# A green gate 1: the file comes back and no process survived.
run_gate pass 1 "$WORK/good" 0 --adapter "$WORK/adapter" --sandbox sb-1 \
    --runtime-digest deadbeef --host host-11 --gate 1 --evidence "$WORK/good"
[[ -s "$WORK/good/persistence-gate-1.txt" ]] || {
    printf 'the gate wrote no evidence file\n' >&2
    exit 1
}
grep -q '^result=pass$' "$WORK/good/persistence-gate-1.txt" || {
    printf 'the evidence did not record a pass\n' >&2
    exit 1
}

# A surviving process turns gate 1 red.
run_gate fail 1 "$WORK/survivor" 1 --adapter "$WORK/adapter" --sandbox sb-1 \
    --runtime-digest deadbeef --host host-11 --gate 1

# A lost file turns gate 1 red even when every call succeeded: the adapter accepts every call
# and answers the read with nothing, which is what a start that did not reattach the volume looks
# like from the outside.
run_gate fail 1 "$WORK/lost" 0 --adapter /bin/true --sandbox sb-1 \
    --runtime-digest deadbeef --host host-11 --gate 1

# A green gate 3: exec answers and the file is present after the api restart.
run_gate pass 3 "$WORK/good" 0 --adapter "$WORK/adapter" --sandbox sb-1 \
    --runtime-digest deadbeef --host host-11 --gate 3

# A missing adapter is a usage error, not a red gate.
status=0
"$GATE" --adapter "$WORK/nope" --sandbox sb-1 --runtime-digest deadbeef --host host-11 --gate 1 \
    >/dev/null 2>&1 || status=$?
[[ "$status" -eq 2 ]] || {
    printf 'a missing adapter should exit 2, got %s\n' "$status" >&2
    exit 1
}

# An unknown gate is a usage error.
status=0
"$GATE" --adapter "$WORK/adapter" --sandbox sb-1 --runtime-digest deadbeef --host host-11 --gate 9 \
    >/dev/null 2>&1 || status=$?
[[ "$status" -eq 2 ]] || {
    printf 'an unknown gate should exit 2, got %s\n' "$status" >&2
    exit 1
}

printf 'persistence-gate contract: all cases passed\n'
