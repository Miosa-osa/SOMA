#!/usr/bin/env bash
#
# Proves the two pieces of the end to end check that decide pass from fail.
#
# The check's value is that a failure is never swallowed, so the judgement it applies to a run
# envelope, and the truncation it applies to a detail string, are worth holding still. The
# fixture is a real `soma run --format json` envelope from a KVM host; the failing cases are
# derived from it so each one differs from a passing run in exactly one field.

set -euo pipefail

TEST_REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
readonly TEST_REPO_ROOT
readonly INSPECT="$TEST_REPO_ROOT/scripts/end-to-end/inspect-run.py"
readonly GOOD="$TEST_REPO_ROOT/scripts/tests/fixtures/run-envelope.json"

FIXTURE_WORK="$(mktemp -d)"
readonly FIXTURE_WORK
trap 'rm -rf -- "$FIXTURE_WORK"' EXIT

# shellcheck disable=SC1091
source "$TEST_REPO_ROOT/scripts/end-to-end-check.sh"

expect_bound() {
    local actual
    actual="$(printf '%s' "$1" | bound)"
    if [[ "$actual" != "$2" ]]; then
        printf 'bound gave %s, expected %s\n' "$actual" "$2" >&2
        exit 1
    fi
}

expect_bound 'a	b
c' 'a b c'
expect_bound 'say "no" \ok' 'say \"no\" \\ok'
if (( $(printf 'x%.0s' {1..400} | bound | wc -c) != 201 )); then
    printf 'bound did not hold the detail string to 200 characters\n' >&2
    exit 1
fi

expect_run() {
    local name="$1" expected="$2" actual status=0
    actual="$(python3 "$INSPECT" "$FIXTURE_WORK/$name.json" '^v[0-9]')" || status=$?
    if [[ "$expected" == pass && "$status" -ne 0 ]]; then
        printf '%s should have passed, got: %s\n' "$name" "$actual" >&2
        exit 1
    fi
    if [[ "$expected" != pass && "$status" -eq 0 ]]; then
        printf '%s should have failed, got: %s\n' "$name" "$actual" >&2
        exit 1
    fi
    if [[ "$expected" != pass && "$actual" != *"$expected"* ]]; then
        printf '%s failed for the wrong reason: %s\n' "$name" "$actual" >&2
        exit 1
    fi
}

python3 - "$GOOD" "$FIXTURE_WORK" <<'PY'
import base64, json, sys

good, work = sys.argv[1], sys.argv[2]
with open(good, "r", encoding="utf-8") as handle:
    original = json.load(handle)


def variant(name, mutate):
    record = json.loads(json.dumps(original))
    mutate(record)
    with open(f"{work}/{name}.json", "w", encoding="utf-8") as out:
        out.write(json.dumps(record) + "\n")


variant("good", lambda record: None)
variant("refused", lambda record: record.update({"status": "error"}))
variant("nonzero", lambda record: record["result"]["execution"].update({"exited": {"code": 3}}))
variant(
    "wrong_output",
    lambda record: record["result"]["stdout"].update(
        {"data": base64.b64encode(b"not a version").decode()}
    ),
)
variant("leaked", lambda record: record["receipt"]["cleanup"].update({"storage": "incomplete"}))
variant(
    "leaked_network",
    lambda record: record["receipt"]["cleanup"]["network"].update({"lease": "incomplete"}),
)
with open(f"{work}/empty.json", "w", encoding="utf-8") as out:
    out.write("")
PY

expect_run good pass
expect_run refused "the run reported error"
expect_run nonzero "did not exit zero"
expect_run wrong_output "does not match"
expect_run leaked "storage=incomplete"
expect_run leaked_network "network.lease=incomplete"
expect_run empty "wrote no envelope"

printf 'end to end check contract passed\n'
