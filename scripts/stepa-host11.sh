#!/usr/bin/env bash
#
# Every variable in this file is expanded HERE, on the client, on purpose: the strings that reach
# the jump host and the host are built from local values. The single-quoted remote strings are
# deliberate for the same reason.
# shellcheck disable=SC2029,SC2016
#
# Runs the ADR 0053 Step A gate sequence and files its evidence under tasks/soma-speed/reports/ in
# the miosa repository. Run it from the workstation or from compute-01; every host action goes
# over ssh through the jump host.
#
# The stages, and why they are in this order:
#   0. preflight on host-11, which refuses unless the installed bundle carries the two changes the
#      sequence depends on: SOMA #2146, without which the drop-in kills the machine subtree on a
#      stop, so a gate that restarts soma-api measures nothing; and SOMA #2154, without which
#      soma-hostd is not installed and there is no volume store at all. It also refuses a daemon
#      with no socket or no host key, and an API whose environment has no SOMA_HOSTD_SOCKET.
#   1. recapture a persistent Generation. This runs on the BUILD host, not on host-11, because the
#      tools take an OCI layout, the built kernel and the guest agent, which live only in the
#      build tree: `scripts/prepare-generation.sh` with SOMA_PREPARE_PERSISTENCE=1, then
#      `capture_snapshot`. Skip it with --skip-recapture when the entry already exists.
#   2. carry that entry into host-11's store and install it with `install-host.sh generations`.
#   3. attach a volume by hand, then create the sandbox naming the reference and the volume. The
#      hand attach is what makes the gates runnable before the control plane's own attach exists.
#   4. the three gate tests of spec section 16, through scripts/persistence-gate.sh with an adapter
#      this file generates, and the report beside them.
#
# Usage:
#   scripts/stepa-host11.sh [--gate N] [--skip-recapture] [--reference REF] [--report-dir DIR]
#                           [--instance ID] [--volume ID] [--build-host NAME]
#                           [--image-source skopeo-SOURCE] [--image-archive PATH]
#
# The sequence refuses, rather than guessing: a reference the store already holds (two entries
# under one reference make `prepared::find` report Ambiguous, and then nothing resolves), a tenant
# that is not the QA tenant, and a protected host name.

set -euo pipefail

JUMP="${SOMA_JUMP:-compute}"
HOST="${SOMA_GATE_HOST:-miosa-host-11.fleet.miosa.internal}"
BUILD="${SOMA_BUILD_HOST:-miosa-host-10.fleet.miosa.internal}"
BUILD_TREE="${SOMA_BUILD_TREE:-/srv/miosa/build/soma-persist}"
# The reference names the ENTRY, not an image to pull, so the recapture needs the image it is built
# from named separately. It is exported with `docker save` rather than read from the docker daemon
# through skopeo: skopeo 1.13.3 speaks client API 1.41 and Docker 29 refuses anything below 1.44
# ("client version 1.41 is too old"), while a docker archive needs no daemon API at all.
IMAGE_NAME="${SOMA_GATE_IMAGE_NAME:-soma-browser-default:2}"
IMAGE_ARCHIVE="${SOMA_GATE_IMAGE_ARCHIVE:-${BUILD_TREE}/soma-browser-default-2.tar}"
IMAGE_SOURCE="${SOMA_GATE_IMAGE_SOURCE:-}"
FS_TOOLS="${SOMA_GATE_FS_TOOLS:-/srv/soma/fs-tools}"
INPUT="${SOMA_INPUT:-/srv/miosa/soma-any-size-install}"
URL="${SOMA_GATE_URL:-127.0.0.1:18787}"
TENANT="${SOMA_GATE_TENANT:-step-a-cert}"
REFERENCE="${SOMA_GATE_REFERENCE:-soma-default:G10-stepa}"
SET="${SOMA_GATE_SET:-stepa-G10}"
TOOLS="${SOMA_GATE_TOOLS:-/srv/soma/stepa-tools}"
SHAPE_MIB=2048
SHAPE_DISK=10240
GATE=0
SKIP_RECAPTURE=0
INSTANCE=""
VOLUME=""
REPORT_DIR=""

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIR
# The gates run on host-11, the LUKS host the spec names, and never on 03, 04 or 10, which serve
# customers and have no encrypted data mount for a persistent volume.
NEVER=(miosa-host-03 miosa-host-04 miosa-host-10)

die() { printf 'stepa-host11: %s\n' "$1" >&2; exit 2; }
say() { printf '\n==> %s\n' "$1"; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        --gate) GATE="${2:-}"; shift 2 ;;
        --skip-recapture) SKIP_RECAPTURE=1; shift ;;
        --reference) REFERENCE="${2:-}"; shift 2 ;;
        --report-dir) REPORT_DIR="${2:-}"; shift 2 ;;
        --instance) INSTANCE="${2:-}"; shift 2 ;;
        --volume) VOLUME="${2:-}"; shift 2 ;;
        --build-host) BUILD="${2:-}"; shift 2 ;;
        --image-source) IMAGE_SOURCE="${2:-}"; shift 2 ;;
        --image-archive) IMAGE_ARCHIVE="${2:-}"; shift 2 ;;
        -h|--help) sed -n '8,36p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) die "unknown argument: $1" ;;
    esac
done
[[ "${GATE}" =~ ^[0-3]$ ]] || die "--gate must be 0, 1, 2 or 3"
[[ "${TENANT}" == "step-a-cert" ]] || die "these gates run on the QA tenant only, not ${TENANT}"
for name in "${NEVER[@]}"; do
    [[ "${HOST%%.*}" != "${name}" ]] || die "${HOST} is a protected production host; the gates run on host-11 only"
done
[[ "${HOST%%.*}" == "miosa-host-11" ]] || die "${HOST} is not host-11, the LUKS host these gates certify"
if [[ -z "${REPORT_DIR}" ]]; then
    REPORT_DIR="${MIOSA_REPO:-/Users/rhl/code/MIOSA/code/miosa}/tasks/soma-speed/reports"
fi
[[ -d "${REPORT_DIR}" ]] || die "no report directory ${REPORT_DIR}; create it or pass --report-dir"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
EVIDENCE="${REPORT_DIR}/stepa-host11-${STAMP}"

# The jump host carries every host command; two ssh hops, one command each.
on_host() { ssh "${JUMP}" "sudo -n ssh -o BatchMode=yes root@${HOST} $1"; }
on_build() { ssh "${JUMP}" "sudo -n ssh -o BatchMode=yes root@${BUILD} $1"; }
# One HTTP call to the host's own soma-api, with the JSON body on stdin.
api() {
    local method="$1" path="$2"
    ssh "${JUMP}" "sudo -n ssh -o BatchMode=yes root@${HOST} 'curl -sS -m 300 -w \"\\\\n%{http_code}\" -H \"x-soma-tenant: ${TENANT}\" -H \"content-type: application/json\" -X ${method} --data-binary @- http://${URL}${path}'"
}

# ---------------------------------------------------------------- 0. preflight

say "preflight on ${HOST}"
on_host "test -f ${INPUT}/install-host.sh" || die "no ${INPUT}/install-host.sh: push the bundle first"
on_host "grep -q '^cmd_hostd()' ${INPUT}/install-host.sh" \
    || die "the installed bundle predates SOMA #2154: it has no hostd step, so there is no volume store"
# The installed drop-in is what can kill a machine, not the installer's text: the installer names
# cgroup.kill in the assertion that checks for it, so grepping the script refuses a correct bundle.
on_host "test -f /etc/systemd/system/soma-api.service.d/60-soma-live-cgroup.conf" \
    || die "no soma-live drop-in on ${HOST}: the cgroup step has not run there"
on_host "grep -l cgroup.kill /etc/systemd/system/soma-api.service.d/*.conf 2>/dev/null && exit 1 || exit 0" \
    || die "an installed drop-in kills the machine subtree on a stop: SOMA #2146 is not applied on ${HOST}"
on_host "systemctl is-active --quiet soma-hostd.service" || die "soma-hostd is not running"
on_host "test -S /run/soma-hostd/allocator.sock" || die "soma-hostd has no control socket"
on_host "test \"\$(stat -c %s /srv/miosa-enc/soma/volume-master.key)\" = 32" || die "the host key is missing or not 32 bytes"
on_host "systemctl show soma-api -p Environment --value | grep -q SOMA_HOSTD_SOCKET" \
    || die "the API has no SOMA_HOSTD_SOCKET in its environment: restart soma-api first"
say "preflight ok"

# ---------------------------------------------------------------- 1. recapture

if [[ "${SKIP_RECAPTURE}" == 0 ]]; then
    say "recapture prerequisites on ${BUILD}"
    if [[ -z "${IMAGE_SOURCE}" ]]; then
        on_build "test -s ${IMAGE_ARCHIVE}" || on_build "docker save ${IMAGE_NAME} -o ${IMAGE_ARCHIVE}" \
            || die "${BUILD} carries no ${IMAGE_NAME} to export (docker save failed there)"
        IMAGE_SOURCE="docker-archive:${IMAGE_ARCHIVE}"
    fi
    on_build "ls ${BUILD_TREE}/step-a-src/kernel/out/vmlinux-* >/dev/null 2>&1" \
        || die "no built kernel under ${BUILD_TREE}/step-a-src/kernel/out on ${BUILD} (run kernel/build.sh)"
    on_build "test -d ${FS_TOOLS}/erofs && test -d ${FS_TOOLS}/e2fsprogs" \
        || die "no filesystem tools in ${FS_TOOLS} on ${BUILD} (run scripts/build-fs-tools.sh)"
    on_build "test -x ${BUILD_TREE}/step-a-src/target/x86_64-unknown-linux-musl/release/soma-guest-agent" \
        || die "no static guest agent on ${BUILD} (run scripts/build-guest-agent.sh)"
    say "recapturing a persistent Generation as ${REFERENCE} on ${BUILD}"
    on_host "test ! -e \$(awk -F= '/^SOMA_GENERATION_STORE=/{print \$2}' /etc/soma/api.env)/ref-\$(printf %s '${REFERENCE}' | sha256sum | cut -d' ' -f1)" \
        || die "${HOST}'s store already holds ${REFERENCE}; pick another --reference"
    on_build "test -d ${BUILD_TREE}/step-a-src" \
        || die "no ${BUILD_TREE}/step-a-src on ${BUILD}: the recapture needs the build tree"
    on_build "cd ${BUILD_TREE}/step-a-src && SOMA_PREPARE_PERSISTENCE=1 SOMA_IMAGE_SOURCE=${IMAGE_SOURCE} scripts/prepare-generation.sh ${REFERENCE} /srv/soma/stepa-prepared ${FS_TOOLS} ${SHAPE_MIB} ${SHAPE_DISK}" \
        || die "the recapture failed; the tool's output above is the reason"
    on_build "cd ${BUILD_TREE}/step-a-src && ./target/release/examples/capture_snapshot /srv/soma/stepa-prepared/ref-\$(printf %s '${REFERENCE}' | sha256sum | cut -d' ' -f1) ${SHAPE_MIB}" \
        || die "the capture failed; a Generation with no snapshot refuses to launch"
    say "recaptured and captured"
fi

# ---------------------------------------------------------------- 2. deliver the entry

say "delivering the entry to ${HOST}'s store"
ENTRY="ref-$(printf %s "${REFERENCE}" | shasum -a 256 | cut -d' ' -f1)"
STORE_REF="$(on_host "cat \$(awk -F= '/^SOMA_GENERATION_STORE=/{print \$2}' /etc/soma/api.env)/${ENTRY}/reference 2>/dev/null" | tr -d '\r')"
if [[ "${STORE_REF}" != "${REFERENCE}" ]]; then
    ssh "${JUMP}" "sudo -n ssh -o BatchMode=yes root@${BUILD} 'tar --sparse -C /srv/soma/stepa-prepared -cf - ${ENTRY}'" \
        | ssh "${JUMP}" "sudo -n ssh -o BatchMode=yes root@${HOST} 'mkdir -p ${INPUT}/generations/${SET} && tar -xf - -C ${INPUT}/generations/${SET}'" \
        || die "could not carry ${ENTRY} to ${HOST}"
    on_host "bash ${INPUT}/install-host.sh generations" || die "install-host.sh generations refused"
fi
on_host "test -f \$(awk -F= '/^SOMA_GENERATION_STORE=/{print \$2}' /etc/soma/api.env)/${ENTRY}/reference" \
    || die "${ENTRY} is not in ${HOST}'s store"
say "the store holds ${REFERENCE}"

# ---------------------------------------------------------------- 3. attach and create

GENERATION="$(on_host "cat \$(awk -F= '/^SOMA_GENERATION_STORE=/{print \$2}' /etc/soma/api.env)/${ENTRY}/generation.id" | tr -d '\r')"
[[ "${GENERATION}" =~ ^[0-9a-f]{64}$ ]] || die "could not read the Generation id (got '${GENERATION}')"
[[ -n "${VOLUME}" ]] || VOLUME="$(on_host 'openssl rand -hex 16' | tr -d '\r')"
[[ -n "${INSTANCE}" ]] || INSTANCE="$(on_host 'uuidgen | tr -d - | tr "A-F" "a-f"' | tr -d '\r')"
[[ "${VOLUME}" =~ ^[0-9a-f]{32}$ ]] || die "--volume must be 32 lowercase hex"
[[ "${INSTANCE}" =~ ^[0-9a-f]{32}$ ]] || die "--instance must be 32 lowercase hex"

say "attaching volume ${VOLUME} to instance ${INSTANCE}"
ATTACH="$(printf '{"operation_id":"%s","instance_id":"%s","volume_id":"%s","request_fingerprint":"%s","size_class_gib":%s,"generation_id":"%s","tenant":"%s"}' \
    "$(on_host 'openssl rand -hex 16' | tr -d '\r')" "${INSTANCE}" "${VOLUME}" \
    "$(on_host 'openssl rand -hex 32' | tr -d '\r')" "$((SHAPE_DISK / 1024))" "${GENERATION}" "${TENANT}" | api POST /v1/volumes/attach)"
printf '%s\n' "${ATTACH}" | sed '$d' >"${EVIDENCE}.attach.json"
[[ "$(printf '%s' "${ATTACH}" | tail -1)" == "200" ]] || die "the attach was refused: $(printf '%s' "${ATTACH}" | sed '$d' | head -c 300)"
say "volume attached"

say "creating the persistent sandbox"
CREATE="$(printf '{"image":"%s","instance_id":"%s","operation_id":"%s","volume_id":"%s","shape":{"vcpu_count":8,"memory_mib":%s,"storage_mib":%s,"capabilities":{"network":{"profile":{"mode":"disabled"},"guest_addresses":{"ipv4":{"mode":"disabled"},"ipv6":{"mode":"disabled"}},"proxy":{"mode":"disabled"},"egress":"denied","dns":{"mode":"denied"},"published_ports":[]}}}}' \
    "${REFERENCE}" "${INSTANCE}" "$(on_host 'openssl rand -hex 32' | tr -d '\r')" "${VOLUME}" "${SHAPE_MIB}" "${SHAPE_DISK}" | api POST /v1/sandboxes)"
printf '%s\n' "${CREATE}" | sed '$d' >"${EVIDENCE}.create.json"
CODE="$(printf '%s' "${CREATE}" | tail -1)"
[[ "${CODE}" == "200" || "${CODE}" == "201" ]] || die "the create was refused with http ${CODE}: $(printf '%s' "${CREATE}" | sed '$d' | head -c 300)"
say "sandbox ${INSTANCE} created with volume ${VOLUME}"

# ---------------------------------------------------------------- 4. the gates

HELPER="${EVIDENCE}.adapter.sh"
cat >"${HELPER}" <<ADAPTER
#!/usr/bin/env bash
# The adapter the gate harness calls. Every call is a public soma-api request on ${HOST}, through
# the jump host; the helper below builds the JSON on the host so nothing is quoted twice.
set -euo pipefail
JUMP="${JUMP}"; HOST="${HOST}"; TOOLS="${TOOLS}"; TENANT="${TENANT}"; REFERENCE="${REFERENCE}"; VOLUME="${VOLUME}"
SHAPE='{"vcpu_count":8,"memory_mib":2048,"storage_mib":10240,"capabilities":{"network":{"profile":{"mode":"disabled"},"guest_addresses":{"ipv4":{"mode":"disabled"},"ipv6":{"mode":"disabled"}},"proxy":{"mode":"disabled"},"egress":"denied","dns":{"mode":"denied"},"published_ports":[]}}}'
case "\$1" in
    write) printf '%s' "\$4" | ssh "\${JUMP}" "sudo -n ssh -o BatchMode=yes root@\${HOST} '\${TOOLS}/stepa-api.sh write-stdin \$2 \$3'" ;;
    read)  ssh "\${JUMP}" "sudo -n ssh -o BatchMode=yes root@\${HOST} '\${TOOLS}/stepa-api.sh read \$2 \$3'" ;;
    exec)  printf '%s' "\$3" | ssh "\${JUMP}" "sudo -n ssh -o BatchMode=yes root@\${HOST} '\${TOOLS}/stepa-api.sh exec-stdin \$2'" ;;
    stop)  ssh "\${JUMP}" "sudo -n ssh -o BatchMode=yes root@\${HOST} '\${TOOLS}/stepa-api.sh delete \$2'" ;;
    start) ssh "\${JUMP}" "sudo -n ssh -o BatchMode=yes root@\${HOST} '\${TOOLS}/stepa-api.sh create \$2 \${REFERENCE} \${VOLUME} \${SHAPE}'" ;;
    reboot) ssh "\${JUMP}" "sudo -n ssh -o BatchMode=yes root@\${HOST} 'systemctl reboot'" >/dev/null 2>&1 || true
            sleep 30
            ssh "\${JUMP}" "sudo -n ssh -o BatchMode=yes root@\${HOST} 'systemctl is-system-running --wait'" >/dev/null ;;
    api-restart) ssh "\${JUMP}" "sudo -n ssh -o BatchMode=yes root@\${HOST} 'systemctl restart soma-api.service && systemctl is-active --wait soma-api.service'" >/dev/null ;;
    *) printf 'unknown adapter call %s\n' "\$1" >&2; exit 2 ;;
esac
ADAPTER
chmod 0755 "${HELPER}"

cat >"${EVIDENCE}.helper.sh" <<'HELPER'
#!/usr/bin/env bash
# Runs on the host: the four sandbox calls the gate harness needs, against the local API.
set -euo pipefail
URL="${SOMA_GATE_URL:-127.0.0.1:18787}"
TENANT="${SOMA_GATE_TENANT:-step-a-cert}"
command_() {
    local instance="$1" script="$2" body answer
    body="$(printf '{"executable":"/bin/sh","arguments":["-c",%s],"operation_id":"%s"}' \
        "$(printf '%s' "${script}" | python3 -c 'import json,sys; sys.stdout.write(json.dumps(sys.stdin.read()))')" \
        "$(openssl rand -hex 16)")"
    answer="$(curl -sS -m 300 -w '\n%{http_code}' -H "x-soma-tenant: ${TENANT}" -H 'content-type: application/json' \
        -X POST "http://${URL}/v1/sandboxes/${instance}/commands" -d "${body}")"
    [[ "$(printf '%s' "${answer}" | tail -1)" == "200" ]] || { printf '%s' "${answer}" | sed '$d' >&2; return 1; }
    printf '%s' "${answer}" | sed '$d' | python3 -c '
import base64, json, sys
d = json.load(sys.stdin); r = d.get("result") if isinstance(d, dict) else None; r = r if isinstance(r, dict) else {}
o = r.get("stdout") or {}
sys.stdout.write(base64.b64decode(o.get("data","")).decode("utf-8","replace") if o.get("encoding") == "base64" else (o.get("data") or ""))
'
}
case "$1" in
    write-stdin) payload="$(cat)"; command_ "$2" "printf '%s' $(printf '%q' "${payload}") > $(printf '%q' "$3")" >/dev/null ;;
    exec-stdin) script="$(cat)"; command_ "$2" "${script}" ;;
    delete) curl -sS -m 300 -H "x-soma-tenant: ${TENANT}" -X DELETE "http://${URL}/v1/sandboxes/$2" >/dev/null ;;
    create) curl -sS -m 300 -H "x-soma-tenant: ${TENANT}" -H 'content-type: application/json' -X POST "http://${URL}/v1/sandboxes" \
                -d "{\"image\":\"$3\",\"instance_id\":\"$2\",\"operation_id\":\"$(openssl rand -hex 32)\",\"volume_id\":\"$4\",\"shape\":$5}" >/dev/null ;;
    *) printf 'unknown helper call %s\n' "$1" >&2; exit 2 ;;
esac
HELPER
ssh "${JUMP}" "sudo -n ssh -o BatchMode=yes root@${HOST} 'mkdir -p ${TOOLS} && cat > ${TOOLS}/stepa-api.sh && chmod 0755 ${TOOLS}/stepa-api.sh'" <"${EVIDENCE}.helper.sh" \
    || die "could not place the API helper on ${HOST}"

say "running the gate harness"
"${SCRIPT_DIR}/persistence-gate.sh" --adapter "${HELPER}" --sandbox "${INSTANCE}" \
    --runtime-digest "$(on_host "sha256sum ${INPUT}/bin/soma-api | cut -d' ' -f1" | tr -d '\r')" \
    --host "${HOST}" --gate "${GATE}" --evidence "${EVIDENCE}" | tee "${EVIDENCE}.log"

{
    printf '# Step A gates on host-11, %s\n\n' "${STAMP}"
    printf -- '- host: %s\n- runtime digest: %s\n- Generation reference: %s\n- Generation id: %s\n- volume: %s\n- instance: %s\n- tenant: %s\n\n' \
        "${HOST}" "$(on_host "sha256sum ${INPUT}/bin/soma-api | cut -d' ' -f1" | tr -d '\r')" \
        "${REFERENCE}" "${GENERATION}" "${VOLUME}" "${INSTANCE}" "${TENANT}"
    printf 'Raw output: `%s.log`, `.attach.json`, `.create.json`; the adapter and the host helper are\nbeside them as `%s.adapter.sh` and `%s.helper.sh`.\n\n' "${EVIDENCE##*/}" "${EVIDENCE##*/}" "${EVIDENCE##*/}"
    printf '## The harness verdict\n\n```\n'
    tail -14 "${EVIDENCE}.log"
    printf '```\n'
} >"${EVIDENCE}.md"
rm -f "${EVIDENCE}.helper.sh"
say "report written to ${EVIDENCE}.md"
