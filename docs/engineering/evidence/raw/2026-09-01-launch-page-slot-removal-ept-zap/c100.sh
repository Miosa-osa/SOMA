#!/usr/bin/env bash
set -uo pipefail
cd /srv/soma/ept
for r in 1 2 3 4; do
    ./cohort.sh 100 c100-off-$r
    SOMA_KVM_DEFER_LAUNCH_PAGE_SLOT=1 ./cohort.sh 100 c100-on-$r
done
df -h /srv | tail -1
echo C100_DONE
