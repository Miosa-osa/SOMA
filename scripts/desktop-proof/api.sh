#!/usr/bin/env bash
# shellcheck disable=SC2034
#
# The calls the desktop proof makes to soma-api's loopback face, sourced by run.sh.
#
# Every exchange is kept: the request body and the response envelope are written under $OUT with
# a step number, so the evidence is the exact bytes that crossed the API, not a summary of them.

API="${SOMA_PROOF_API:-127.0.0.1:18930}"
TENANT="${SOMA_PROOF_TENANT:-m1}"
STEP=0
LAST=""     # path of the last response envelope
STATUS=""   # its HTTP status
SECONDS_TAKEN=""

# api <name> <method> <path> [body]: one call, kept as $OUT/NN-name.{request,response}.
api() {
    local name="$1" method="$2" path="$3" body="${4-}"
    STEP=$((STEP + 1))
    local stem
    stem="$(printf '%s/%02d-%s' "$OUT" "$STEP" "$name")"
    printf '%s %s\n\n%s\n' "$method" "$path" "${body:-<no body>}" >"$stem.request"
    local -a data=()
    [[ -n "$body" ]] && data=(--data-binary "$body")
    local timing
    timing="$(curl -sS -o "$stem.response" -w '%{http_code} %{time_total}' -m 600 \
        -X "$method" "http://$API$path" -H "x-soma-tenant: $TENANT" \
        -H 'content-type: application/json' "${data[@]}")"
    STATUS="${timing%% *}"
    SECONDS_TAKEN="${timing#* }"
    LAST="$stem.response"
    printf '%s %s -> %s in %s s\n' "$method" "$path" "$STATUS" "$SECONDS_TAKEN" | tee -a "$OUT/calls.log"
}

# field <python expression on d> [file]: one value out of a JSON file (the last response by default).
field() {
    python3 -c 'import json,sys; d=json.load(open(sys.argv[2])); print(eval(sys.argv[1]))' "$1" "${2:-$LAST}"
}

# guest <name> <executable> [argument...]: runs one command in the sandbox through exec.
# The decoded stdout goes to $OUT/NN-name.stdout and stderr to .stderr; the exit code is GUEST_EXIT.
guest() {
    local name="$1" executable="$2"
    shift 2
    local body
    body="$(python3 -c 'import json,sys; print(json.dumps({"executable": sys.argv[1], "arguments": sys.argv[2:], "limits": {"timeout_ms": 120000, "max_output_bytes": 8388608}}))' "$executable" "$@")"
    api "$name" POST "/v1/sandboxes/$INSTANCE/commands" "$body"
    python3 - "$LAST" "${LAST%.response}" <<'PY'
import base64, json, sys
envelope = json.load(open(sys.argv[1]))
result = envelope.get("result") or {}
for stream in ("stdout", "stderr"):
    data = (result.get(stream) or {}).get("data", "")
    open(f"{sys.argv[2]}.{stream}", "wb").write(base64.b64decode(data))
PY
    GUEST_EXIT="$(field 'd["result"]["execution"].get("exited", {}).get("code", "none") if d.get("result") else "error"')"
    GUEST_OUT="${LAST%.response}.stdout"
    if [[ "$STATUS" != 200 || "$GUEST_EXIT" != 0 ]]; then
        printf 'guest step %s failed (HTTP %s, exit %s); see %s\n' \
            "$name" "$STATUS" "$GUEST_EXIT" "$LAST" >&2
        return 1
    fi
}

# fetch <guest path> <local file>: reads one file out of the sandbox through the filesystem API.
fetch() {
    api "read-$(basename "$2")" POST "/v1/sandboxes/$INSTANCE/filesystem/read" "{\"path\":\"$1\"}"
    python3 - "$LAST" "$2" <<'PY'
import base64, json, sys
envelope = json.load(open(sys.argv[1]))
result = envelope.get("result") or {}
content = result.get("content")
if envelope.get("status") != "ok" or not isinstance(content, dict) or content.get("encoding") != "base64":
    raise SystemExit(f"guest file read failed; see {sys.argv[1]}")
data = base64.b64decode(content["data"], validate=True)
if len(data) != content.get("byte_length"):
    raise SystemExit(f"guest file read length mismatch; see {sys.argv[1]}")
with open(sys.argv[2], "wb") as output:
    output.write(data)
PY
}

# The network policy that leaves egress to the operator default, so the broker leases a network.
RUNTIME_DEFAULT_NETWORK='{"profile":{"mode":"operator_default"},"guest_addresses":{"ipv4":{"mode":"unspecified"},"ipv6":{"mode":"unspecified"}},"proxy":{"mode":"disabled"},"egress":"unspecified","dns":{"mode":"unspecified"},"published_ports":[]}'
