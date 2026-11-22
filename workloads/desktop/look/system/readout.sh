#!/bin/sh
# The top bar's readout: CPU busy, memory in use, disk used of total, network down and up, one line
# every two seconds, read from /proc and df.
cpu() { awk '/^cpu /{print $2+$3+$4+$7+$8+$9, $5+$6}' /proc/stat; }
net() { awk -F'[: ]+' 'NR>2 && $2!="lo" {rx+=$3; tx+=$11} END {print rx+0, tx+0}' /proc/net/dev; }
rate() { awk -v b="$1" 'BEGIN { if (b >= 1048576) printf "%.1fM", b/1048576; else if (b >= 1024) printf "%.0fK", b/1024; else printf "%dB", b }'; }
# shellcheck disable=SC2046 # the words are the numbers
set -- $(cpu) $(net)
pb=$1 pi=$2 prx=$3 ptx=$4
while sleep 2; do
    # shellcheck disable=SC2046
    set -- $(cpu) $(net)
    db=$(( $1 - pb )); di=$(( $2 - pi )); t=$(( db + di ))
    pct=0; [ "$t" -gt 0 ] && pct=$(( 100 * db / t ))
    rx=$(( ($3 - prx) / 2 )); tx=$(( ($4 - ptx) / 2 ))
    pb=$1 pi=$2 prx=$3 ptx=$4
    mem=$(awk '/MemTotal/{t=$2}/MemAvailable/{a=$2}END{printf "%.1f/%.0fG", (t-a)/1048576, t/1048576}' /proc/meminfo)
    disk=$(df -BG --output=used,size / | awk 'NR==2{gsub("G","",$1); gsub("G","",$2); printf "%s/%sG", $1, $2}')
    printf 'CPU %s%%   RAM %s   DISK %s   NET ↓%s ↑%s\n' "$pct" "$mem" "$disk" "$(rate "$rx")" "$(rate "$tx")"
done
