#!/usr/bin/env bash
# Alternating arms through the retained cohort harness, one cohort at a time round robin.
#
# Three arms differing only in how many head directories and how many independent template
# copies a launch may use:
#
#   base    one head directory, no fan: the shape every earlier measurement was taken on
#   shard   sixteen head directories, no fan
#   fan1ag  sixteen head directories and four copies that share one allocation group
#   fanag   sixteen head directories and four copies in four allocation groups
#
# Each arm has its own head root so no arm inherits another's directories. The fan arms point at
# a warmed fan; the others point at an empty directory, which the launcher treats as no fan. The
# one group fan is the four group fan's copies reached through a directory of symbolic links, so
# both arms clone the same four files and differ only in which allocation groups they occupy.
#
# The host is shared, so every cohort waits for a quiet host measured from /proc/stat rather than
# from the load average, and both figures are recorded on both sides of every cohort. The wait is
# per cohort rather than per round because a cohort of a hundred sandboxes is still tearing itself
# down when the next arm starts, and a wait per round would leave every arm sitting in a different
# part of its neighbour's tail.
set -uo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/hostbusy.sh"

N="${1:-40}"
TAG="${2:-fanned}"
LIMIT="${3:-12}"
SETTLE="${4:-3}"
STORE="${STORE:-/srv/soma/s-rw}"
ROOT=/srv/soma/hc
R="$ROOT/raw/$TAG"
mkdir -p "$R" "$ROOT/empty-fan"

cohort() {
    local arm="$1" index="$2"
    local out="$R/$arm-$(printf %02d "$index").jsonl"
    local busy load
    case "$arm" in
        base) export SOMA_HEAD_DIR="$ROOT/heads-base" SOMA_HEAD_SHARDS=1 \
                     SOMA_TEMPLATE_FAN_DIR="$ROOT/empty-fan" SOMA_TEMPLATE_COPIES=1 ;;
        shard) export SOMA_HEAD_DIR="$ROOT/heads-shard" SOMA_HEAD_SHARDS=16 \
                      SOMA_TEMPLATE_FAN_DIR="$ROOT/empty-fan" SOMA_TEMPLATE_COPIES=1 ;;
        fan1ag) export SOMA_HEAD_DIR="$ROOT/heads-fan1ag" SOMA_HEAD_SHARDS=16 \
                       SOMA_TEMPLATE_FAN_DIR="$ROOT/fan-1ag" SOMA_TEMPLATE_COPIES=4 ;;
        fanag) export SOMA_HEAD_DIR="$ROOT/heads-fanag" SOMA_HEAD_SHARDS=16 \
                      SOMA_TEMPLATE_FAN_DIR="$ROOT/fan-ag" SOMA_TEMPLATE_COPIES=4 ;;
        *) printf 'unknown arm %s\n' "$arm" >&2; return 1 ;;
    esac
    wait_quiet "$SETTLE" 600 || return 1
    busy="$(busy_pct 0.5)"
    load="$(cut -d' ' -f1 /proc/loadavg)"
    bash "$(dirname "${BASH_SOURCE[0]}")/hc-tti.sh" "$STORE" 1024 2048 100 "$out" > /dev/null 2>&1
    printf '{"arm":"%s","cohort":%d,"busy_before":%s,"load_before":%s,"busy_after":%s,"at":"%s"}\n' \
        "$arm" "$index" "$busy" "$load" "$(busy_pct 0.5)" "$(date +%T)" >> "$R/load.jsonl"
    printf 'heads-left %s %s %s\n' "$arm" "$index" \
        "$(find "$SOMA_HEAD_DIR" -type f 2>/dev/null | wc -l)" >> "$R/heads.log"
}

# The arm order rotates every round. With a fixed order one arm always runs first after the gate
# and one always runs last, which is a systematic difference between arms rather than a property
# of the arms.
read -r -a ORDER <<< "${ARMS:-base shard fan1ag fanag}"
for ((i = 0; i < N; i++)); do
    wait_quiet "$LIMIT" 1800 || { printf 'gave up waiting for a quiet host\n'; exit 1; }
    for ((slot = 0; slot < ${#ORDER[@]}; slot++)); do
        cohort "${ORDER[(slot + i) % ${#ORDER[@]}]}" "$i"
    done
done
printf 'done %s\n' "$(date +%T)"
