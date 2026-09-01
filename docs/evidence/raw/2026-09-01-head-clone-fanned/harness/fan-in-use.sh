#!/usr/bin/env bash
# Proof that a launch cloned from the fan rather than from the template.
#
# `FICLONE` marks the extents of both files shared, so after a cohort the copies a launch used
# carry the shared flag and a template nothing cloned from does not. The extent maps are taken
# before and after one cohort of the named arm.
set -uo pipefail
TEMPLATE="${1:?template path}"
FAN="${2:?fan directory}"
WHEN="${3:-before}"
OUT="${4:-/srv/soma/hc/raw/fan-in-use}"
mkdir -p "$OUT"
{
    printf '== %s template %s\n' "$WHEN" "$TEMPLATE"
    xfs_io -r -c 'fiemap -v' "$TEMPLATE" | head -5
    for copy in "$FAN"/copy-*; do
        printf '== %s %s\n' "$WHEN" "$copy"
        xfs_io -r -c 'fiemap -v' "$copy" | head -5
    done
} > "$OUT/$WHEN.txt" 2>&1
cat "$OUT/$WHEN.txt"
