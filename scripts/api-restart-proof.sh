#!/usr/bin/env bash
#
# Proves that a machine hosted by soma-api survives a restart of soma-api.
#
# ADR 0042's addendum claims that a claimed machine host leaves the soma-api unit's control group,
# so a stop or restart of the unit no longer SIGKILLs the machines it is serving, and a scratch-unit
# run in a pull request body is not a gate test. This is that proof, scripted and leaving evidence.
#
# What it does, in order:
#   1. creates N machines through the host's own soma-api, at a real Generation and a real shape;
#   2. runs a command on every one, so "it answered before the restart" is recorded too;
#   3. restarts the soma-api unit;
#   4. waits for the API to answer again, on the record-only prewarm route (api-restart-proof/
#      readiness.sh says why that route and not a per-instance one);
#   5. runs a command on every machine again, and fails if any one of them is gone or refuses;
#   6. captures the socket, the host process and the control group of what failed (api-restart-proof/
#      capture.sh), so "gone", "behind a queue" and "killed by a cgroup" are told apart.
#
# It destroys the machines at the end unless SOMA_PROOF_KEEP=1.
#
# Usage:
#   scripts/api-restart-proof.sh [--machines N] [--evidence-dir DIR]
#
# Environment:
#   SOMA_PROOF_MACHINES         number of machines to host (default 8)
#   SOMA_PROOF_IMAGE            a Generation reference; defaults to the first one in the store
#   SOMA_PROOF_SHAPE            "vcpu memory_mib storage_mib" (default "8 2048 10240")
#   SOMA_PROOF_EVIDENCE         evidence directory (default /var/lib/soma-api-restart-proof/<stamp>)
#   SOMA_PROOF_KEEP             1 to leave the machines running, and to capture every machine
#   SOMA_API_UNIT               unit to restart (default soma-api.service)
#   SOMA_API_ENV                env file holding SOMA_GENERATION_STORE (default /etc/soma/api.env)
#   SOMA_PROOF_STATE_ROOT       the runtime state root, when the unit sets one; capture.sh reads
#                               <state root>/machines/<instance>.sock from it
#   SOMA_PROOF_COMMAND_TIMEOUT  client ceiling for one command (default 360)
#   SOMA_PROOF_PROBE_TIMEOUT    client ceiling for one readiness probe (default 150)
#   SOMA_PROOF_READY_TIMEOUT    window in which the API must answer again (default 120)
#
# Run it on the host, as root, after a soma-api start and while the Generation store is admitted.
# This is a disruptive proof: it restarts the service the host is serving from.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIR

MACHINES="${SOMA_PROOF_MACHINES:-8}"
readonly UNIT="${SOMA_API_UNIT:-soma-api.service}"
readonly API_ENV="${SOMA_API_ENV:-/etc/soma/api.env}"
readonly TENANT="${SOMA_PROOF_TENANT:-api-restart-proof}"
readonly KEEP="${SOMA_PROOF_KEEP:-0}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
readonly STAMP
EVIDENCE="${SOMA_PROOF_EVIDENCE:-/var/lib/soma-api-restart-proof/${STAMP}}"
# A client ceiling has to outlast the ceiling the API puts on the same round trip, or a slow but
# legitimate answer is recorded as a client timeout and read as a machine that stopped answering:
# `crates/soma-local/src/backend/kvm/host/mod.rs` allows 330 s for an execute (360 here) and 120 s
# for a control call (one readiness probe gets 150).
readonly COMMAND_TIMEOUT="${SOMA_PROOF_COMMAND_TIMEOUT:-360}"
readonly PROBE_TIMEOUT="${SOMA_PROOF_PROBE_TIMEOUT:-150}"
readonly READY_TIMEOUT="${SOMA_PROOF_READY_TIMEOUT:-120}"
readonly STATE_ROOT="${SOMA_PROOF_STATE_ROOT:-}"

IMAGE="${SOMA_PROOF_IMAGE:-}"

failures=0
created=()

die() { printf '%s\n' "$1" >&2; exit 1; }
stamp() { date -u +%Y-%m-%dT%H:%M:%SZ; }
say() { printf '%s %s\n' "$(stamp)" "$1"; }
record() { printf '%s %s\n' "$(stamp)" "$1" >>"${EVIDENCE}/proof.log"; }

# Both helpers read this script's constants and `record`; neither runs anything until it is called.
# shellcheck source=scripts/api-restart-proof/readiness.sh
source "${SCRIPT_DIR}/api-restart-proof/readiness.sh"
# shellcheck source=scripts/api-restart-proof/capture.sh
source "${SCRIPT_DIR}/api-restart-proof/capture.sh"

usage() {
    # Prints the header comment itself, so the options are documented in exactly one place. The
    # range ends at the first line that is not a comment and drops it, so the header can grow.
    sed -n '3,/^[^#]/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' | sed '$d'
}

for argument in "$@"; do
    case "${argument}" in
        --machines=*) MACHINES="${argument#--machines=}" ;;
        --evidence-dir=*) EVIDENCE="${argument#--evidence-dir=}" ;;
        --help|-h) usage; exit 0 ;;
        *) die "unknown argument: ${argument}" ;;
    esac
done
readonly MACHINES EVIDENCE

[[ "$(id -u)" -eq 0 ]] || die "run as root: this restarts ${UNIT}"
for tool in curl python3 systemctl; do
    command -v "${tool}" >/dev/null 2>&1 || die "required command not found: ${tool}"
done
[[ "${MACHINES}" =~ ^[1-9][0-9]*$ ]] || die "SOMA_PROOF_MACHINES must be a positive integer"

read -r VCPU MEMORY DISK <<<"${SOMA_PROOF_SHAPE:-8 2048 10240}"
for value in "${VCPU}" "${MEMORY}" "${DISK}"; do
    [[ "${value}" =~ ^[1-9][0-9]*$ ]] || die "SOMA_PROOF_SHAPE must be three positive integers"
done

mkdir -p "${EVIDENCE}"
record "proof start on $(hostname); machines=${MACHINES}; unit=${UNIT}"

# The loopback listener is what the host itself serves on, and it is where the unit says it listens.
listen_url() {
    local url
    url="$(grep -ho 'listen [^ ]*' <(systemctl cat "${UNIT}" 2>/dev/null) | head -1 | awk '{print $2}')"
    printf '%s' "${url:-127.0.0.1:18787}"
}
URL="$(listen_url)"
record "api url ${URL}"

# A Generation reference names what a machine boots. The store holds one directory per Generation,
# and each carries the reference the control plane sends.
generation_reference() {
    if [[ -n "${IMAGE}" ]]; then printf '%s' "${IMAGE}"; return; fi
    local store entry ref wanted
    local fallback=""
    store="$(awk -F= '/^SOMA_GENERATION_STORE=/{print $2}' "${API_ENV}" 2>/dev/null || true)"
    [[ -n "${store}" ]] || die "set SOMA_PROOF_IMAGE, or SOMA_GENERATION_STORE in ${API_ENV}"
    # The store keeps one directory per captured Generation and their names are not in age order:
    # host-10's oldest entry still names the legacy "node:22" image, which the runner refuses with
    # workload_rejected. Take the entry whose class matches the shape being launched, and only ever
    # a soma-default generation.
    wanted="soma-default:G$(( DISK / 1024 ))"
    for entry in "${store}"/ref-*/; do
        ref="$(cat "${entry}/reference" 2>/dev/null || true)"
        [[ "${ref}" == soma-default:* ]] || continue
        [[ "${ref}" == "${wanted}"* ]] && { printf '%s' "${ref}"; return; }
        [[ -z "${fallback}" ]] && fallback="${ref}"
    done
    [[ -n "${fallback}" ]] || die "no soma-default Generation under ${store}"
    printf '%s' "${fallback}"
}
REFERENCE="$(generation_reference)"
record "generation reference ${REFERENCE}"

uuid() { tr -d - </proc/sys/kernel/random/uuid; }

# The isolated default shape: no guest network, no published ports. The proof is about the machine
# surviving a restart, so it must not depend on a runner config, an egress policy or netd.

# Runs one command and prints the report. Returns non-zero when the API did not answer 200.
run_command() {
    local instance="$1" marker="$2"
    curl -sS -m "${COMMAND_TIMEOUT}" -w '\n%{http_code}' \
        -H "x-soma-tenant: ${TENANT}" -H 'content-type: application/json' \
        -X POST "http://${URL}/v1/sandboxes/${instance}/commands" \
        -d "{\"executable\":\"/bin/echo\",\"arguments\":[\"${marker}\"],\"operation_id\":\"$(uuid)\"}"
}

# Prints the decoded standard output of a report, or nothing if the report carries none.
decoded_stdout() {
    python3 -c '
import base64, json, sys
try:
    report = json.loads(sys.stdin.read())
except ValueError:
    sys.exit(0)
# The reply is an envelope: the command report is under result, not at the top level.
result = report.get("result") if isinstance(report, dict) else None
if not isinstance(result, dict):
    result = report if isinstance(report, dict) else {}
stdout = result.get("stdout") or {}
if isinstance(stdout, dict) and stdout.get("encoding") == "base64":
    sys.stdout.write(base64.b64decode(stdout.get("data", "")).decode("utf-8", "replace"))
' <<<"$1"
}

# "answers" means the API returned 200 and the guest echoed the marker back. A machine that is gone
# answers with a typed refusal instead, which is exactly what this proof exists to catch.
answers() {
    local instance="$1"
    local marker="proof-${instance}-$$" response body code
    response="$(run_command "${instance}" "${marker}" 2>/dev/null || true)"
    code="$(printf '%s' "${response}" | tail -n1)"
    body="$(printf '%s' "${response}" | sed '$d')"
    if [[ "${code}" != "200" ]]; then
        printf 'http %s: %s' "${code}" "$(printf '%s' "${body}" | head -c 160)"
        return 1
    fi
    if [[ "$(decoded_stdout "${body}")" != *"${marker}"* ]]; then
        printf '200 but stdout did not carry the marker: %s' "$(printf '%s' "${body}" | head -c 160)"
        return 1
    fi
    printf 'ok'
    return 0
}

# ---------------------------------------------------------------- 1. host the machines

say "creating ${MACHINES} machine(s) at ${VCPU} vCPU, ${MEMORY} MiB, ${DISK} MiB, no network"
for _ in $(seq 1 "${MACHINES}"); do
    instance="$(uuid)"
    body="$(printf '{"image":"%s","instance_id":"%s","operation_id":"%s","shape":{"vcpu_count":%s,"memory_mib":%s,"storage_mib":%s,"capabilities":{"network":{"profile":{"mode":"disabled"},"guest_addresses":{"ipv4":{"mode":"disabled"},"ipv6":{"mode":"disabled"}},"proxy":{"mode":"disabled"},"egress":"denied","dns":{"mode":"denied"},"published_ports":[]}}}}' \
        "${REFERENCE}" "${instance}" "$(uuid)" "${VCPU}" "${MEMORY}" "${DISK}")"
    response="$(curl -sS -m 300 -w '\n%{http_code}' -H "x-soma-tenant: ${TENANT}" \
        -H 'content-type: application/json' -X POST "http://${URL}/v1/sandboxes" -d "${body}" 2>/dev/null || true)"
    code="$(printf '%s' "${response}" | tail -n1)"
    printf '%s\n' "${response}" | sed '$d' >"${EVIDENCE}/create-${instance}.json"
    if [[ "${code}" != "200" && "${code}" != "201" ]]; then
        record "create ${instance}: FAILED http ${code}"
        failures=$((failures + 1))
        continue
    fi
    created+=("${instance}")
    record "create ${instance}: ok"
done
[ "${#created[@]}" -gt 0 ] || die "no machine was created; nothing to prove"
printf '%s\n' "${created[@]}" >"${EVIDENCE}/machines.txt"
say "hosted ${#created[@]} machine(s)"

# ---------------------------------------------------------------- 2. they answer before

before_failures=0
for instance in "${created[@]}"; do
    if outcome="$(answers "${instance}")"; then
        record "before ${instance}: ${outcome}"
    else
        record "before ${instance}: FAILED ${outcome}"
        before_failures=$((before_failures + 1))
    fi
done
say "before the restart: $(( ${#created[@]} - before_failures ))/${#created[@]} answered"

# ---------------------------------------------------------------- 3. restart the service

say "restarting ${UNIT}"
record "restart begin $(systemctl show "${UNIT}" -p MainPID --value)"
if ! systemctl restart "${UNIT}"; then
    record "restart: FAILED to run systemctl restart"
    failures=$((failures + 1))
fi
record "restart end $(systemctl show "${UNIT}" -p MainPID --value)"

# ---------------------------------------------------------------- 4. the API answers again

ready=0
if await_api_ready; then
    ready=1
fi
if (( ready == 0 )); then
    say "the API did not answer within ${READY_TIMEOUT}s; the machines cannot be checked"
    failures=$((failures + 1))
fi

# ---------------------------------------------------------------- 5. every machine still answers

if (( ready == 1 )); then
    for instance in "${created[@]}"; do
        if outcome="$(answers "${instance}")"; then
            record "after ${instance}: ${outcome}"
        else
            record "after ${instance}: FAILED ${outcome}"
            failures=$((failures + 1))
        fi
    done
fi

survivors=0
for instance in "${created[@]}"; do
    if grep -q "after ${instance}: ok" "${EVIDENCE}/proof.log"; then
        survivors=$((survivors + 1))
    fi
done
say "after the restart: ${survivors}/${#created[@]} answered"

# ---------------------------------------------------------------- 5b. what a failure was

capture_machines

# ---------------------------------------------------------------- 6. release the machines

if [[ "${KEEP}" != "1" ]]; then
    for instance in "${created[@]}"; do
        curl -sS -m 60 -H "x-soma-tenant: ${TENANT}" -X DELETE \
            "http://${URL}/v1/sandboxes/${instance}" >/dev/null 2>&1 || true
    done
    record "destroyed ${#created[@]} machine(s)"
fi

# ---------------------------------------------------------------- 7. the verdict

record "machines=${#created[@]} survivors=${survivors} failures=${failures}"
printf 'RESULT %s machines=%s survivors=%s failures=%s evidence=%s\n' \
    "$([[ ${failures} -eq 0 && ${survivors} -eq ${#created[@]} ]] && printf pass || printf fail)" \
    "${#created[@]}" "${survivors}" "${failures}" "${EVIDENCE}"
[[ ${failures} -eq 0 && ${survivors} -eq ${#created[@]} ]]
