#!/usr/bin/env bash
#
# Waiting for the API to answer again, on a route that cannot queue on a machine.
#
# Sourced by api-restart-proof.sh, which owns the globals this reads (URL, TENANT, PROBE_TIMEOUT,
# READY_TIMEOUT) and provides `record`.
#
# The probe is deliberately not `GET /v1/sandboxes/<instance>`. That route resolves a per-instance
# record and then probes the machine over its host socket, and a machine host serves one connection
# at a time with a 120 s control ceiling, so a probe whose client gives up early leaves the host
# still working and the probes after it stack up behind the first. The queue then sits in front of
# the first real command, which reads as a machine that is gone when it was only waiting its turn.
# `GET /v1/sandboxes` is not a way out either: it reports liveness by probing every record the same
# way. `GET /v1/prewarm` is the record-only route: it answers this host's own prewarm state, opens
# no sandbox facade and touches no machine socket, so a readiness window cannot queue anything.
#
# The deadline gates starting a probe, not aborting one in flight: a listener that accepted the
# connection and then stalled runs to PROBE_TIMEOUT, which is longer than the 120 s the API allows
# itself, so a restart still draining is never recorded as an API that did not come back.

# Returns 0 once the API answers on the record-only route, 1 when the window closes first. Records
# the outcome either way, so a caller needs no second line.
await_api_ready() {
    local deadline=$(( SECONDS + READY_TIMEOUT ))
    local ready=0
    while (( SECONDS < deadline )); do
        if [[ "$(curl -sS -m "${PROBE_TIMEOUT}" -o /dev/null -w '%{http_code}' \
            -H "x-soma-tenant: ${TENANT}" \
            "http://${URL}/v1/prewarm" 2>/dev/null || true)" == "200" ]]; then
            ready=1
            break
        fi
        sleep 2
    done
    record "api ready after restart: ${ready} (waited ${SECONDS}s of ${READY_TIMEOUT}s)"
    (( ready == 1 ))
}
