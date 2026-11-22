#!/usr/bin/env bash
#
# Compares what run.sh sampled before the sandbox existed with what it sampled after it was
# destroyed, and says, surface by surface, whether anything was left behind.
#
#   residue.sh <results-dir>
#
# processes, netns, nft, links and heads must not gain a single line: a process naming the state
# root, a network namespace, an nftables table, a network link or a file under the heads directory
# that outlived the sandbox is a leak. One exception is counted rather than named: soma-netd keeps
# a pool of prepared, unassigned bundles (a `sv<id>` link and a `somah_<id>` table each) and
# refills it with new ids after every claim, so for those the number must not grow, while every
# other line is still judged by name. The state root is listed rather than judged: soma-api keeps
# durable records there by design, so the entries it gained are printed for the reader.
# Exits 1 when any judged surface leaked.

set -uo pipefail

OUT="${1:?results directory}"
leaked=0
BUNDLE='(^| )(sv|somah_)[0-9a-f]{8}(@|$)'

for surface in processes netns nft links heads; do
    before_bundles="$(grep -cE "$BUNDLE" "$OUT/probe.before.$surface")"
    after_bundles="$(grep -cE "$BUNDLE" "$OUT/probe.after.$surface")"
    added="$(comm -13 <(grep -vE "$BUNDLE" "$OUT/probe.before.$surface") \
        <(grep -vE "$BUNDLE" "$OUT/probe.after.$surface") | head -20)"
    if ((after_bundles > before_bundles)); then
        added+="${added:+$'\n'}bundles: $before_bundles before, $after_bundles after"
    fi
    if [[ -z "$added" ]]; then
        printf '%-9s clean  (%s before, %s after)\n' "$surface" \
            "$(wc -l <"$OUT/probe.before.$surface")" "$(wc -l <"$OUT/probe.after.$surface")"
    else
        printf '%-9s LEAKED:\n%s\n' "$surface" "$added"
        leaked=1
    fi
done

gained="$(comm -13 "$OUT/probe.before.state" "$OUT/probe.after.state")"
printf 'state     %s entries before, %s after; gained:\n' \
    "$(wc -l <"$OUT/probe.before.state")" "$(wc -l <"$OUT/probe.after.state")"
if [[ -n "$gained" ]]; then
    while IFS= read -r entry; do
        if [[ -f "$entry" ]]; then
            printf '  %s (%s bytes)\n' "$entry" "$(stat -c %s "$entry")"
        else
            printf '  %s/\n' "$entry"
        fi
    done <<<"$gained"
else
    printf '  nothing\n'
fi
exit "$leaked"
