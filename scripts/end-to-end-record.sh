#!/usr/bin/env bash
#
# The record end-to-end-check.sh keeps: how a stage is timed and classified, and how the run is
# reported once every stage has been attempted.
#
# It is separated from the stages themselves because the two answer different questions. The
# stages decide what the host did; this decides what is written down about it, and that record is
# what a reader trusts weeks later when the host is gone. Keeping it apart means a change to the
# results schema cannot quietly change what a stage checks, and the reverse.
#
# Sourced by end-to-end-check.sh, which owns the stage functions and the arrays filled here:
# NAMES, STATUSES, DETAILS, DURATIONS, plus DETAIL, FIRST_FAILURE, LOG_DIR, WORK, and IMAGE.

# A detail string goes into a JSON field on one line, so control characters are dropped, tabs and
# newlines become spaces, quotes and backslashes are escaped, and the result is cut to a bound.
bound() {
    tr -d '\000-\010\013\014\016-\037' | tr '\n\t' '  ' \
        | sed 's/\\/\\\\/g; s/"/\\"/g' | cut -c1-200
}

record() {
    NAMES+=("$1"); STATUSES+=("$2"); DETAILS+=("$3"); DURATIONS+=("$4")
    printf '  %-8s %-18s %s\n' "$2" "$1" "$3"
}

# A failure is never worked around: once one stage fails every later stage records skipped, so the
# break stays visible instead of being buried under whatever the remaining stages would have said.
run_stage() {
    local name="$1" fn="$2" started ended status
    DETAIL=""
    if [[ -n "$FIRST_FAILURE" ]]; then
        record "$name" skipped "not attempted after $FIRST_FAILURE failed" 0
        return
    fi
    log "$name"
    started="$(date +%s%3N)"
    if "$fn" >"$LOG_DIR/$name.log" 2>&1; then status=passed; else status=failed; fi
    ended="$(date +%s%3N)"
    [[ -n "$DETAIL" ]] || DETAIL="$(tail -n 1 "$LOG_DIR/$name.log" 2>/dev/null)"
    [[ "$status" == passed ]] || FIRST_FAILURE="$name"
    record "$name" "$status" "$(printf '%s' "$DETAIL" | bound)" "$(( ended - started ))"
}

write_results() {
    local results="$WORK/results.json" index=0 separator="" failure=null
    [[ -z "$FIRST_FAILURE" ]] || failure="\"$FIRST_FAILURE\""
    {
        printf '{"schema":"soma.e2e.v1","image":"%s","host":"%s","commit":"%s",' \
            "$IMAGE" "$(uname -srm | bound)" \
            "${SOMA_E2E_COMMIT:-$(git rev-parse HEAD 2>/dev/null || echo unknown)}"
        printf '"work":"%s","first_failure":%s,"stages":[' "$WORK" "$failure"
        while (( index < ${#NAMES[@]} )); do
            printf '%s{"stage":"%s","status":"%s","detail":"%s","duration_ms":%s,"log":"%s"}' \
                "$separator" "${NAMES[index]}" "${STATUSES[index]}" "${DETAILS[index]}" \
                "${DURATIONS[index]}" "logs/${NAMES[index]}.log"
            separator=","
            index=$(( index + 1 ))
        done
        printf ']}\n'
    } >"$results"
    printf '%s\n' "$results"
}

summarize() {
    log "summary"
    # `read` assigns into the caller's scope unless the name is local, so every name it fills is
    # declared. An undeclared `status` here once silently overwrote main's own exit status.
    local index=0 check outcome detail
    while (( index < ${#NAMES[@]} )); do
        printf '  %-8s %-18s %6s ms  %s\n' "${STATUSES[index]}" "${NAMES[index]}" \
            "${DURATIONS[index]}" "${DETAILS[index]}"
        index=$(( index + 1 ))
    done
    if [[ -f "$WORK/cleanup-checks.tsv" ]]; then
        printf '\n  cleanup checks:\n'
        while IFS=$'\t' read -r check outcome detail; do
            printf '    %-14s %-11s %s\n' "$check" "$outcome" "$detail"
        done <"$WORK/cleanup-checks.tsv"
    fi
    printf '\n  results: %s\n  logs:    %s\n' "$(write_results)" "$LOG_DIR"
    if [[ -n "$FIRST_FAILURE" ]]; then
        printf '\nFAILED at stage %s. Read %s.\n' "$FIRST_FAILURE" "$LOG_DIR/$FIRST_FAILURE.log"
        return 1
    fi
    printf '\nEvery stage passed and the sandbox left nothing behind.\n'
}
