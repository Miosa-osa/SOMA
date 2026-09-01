#!/usr/bin/env bash
set -uo pipefail
cd /srv/soma/ept
sed "s|node-1024-10240|bb-1024-10240|; s|node:22 -- /usr/local/bin/node --version|busybox:stable-musl -- /bin/busybox --help|" cohort.sh > cohort-bb.sh
chmod +x cohort-bb.sh
sed -i "s|\"v22\" not in stdout|False|" /dev/null
REPS=25 ./cohort-bb.sh 1 bb-warm >/dev/null 2>&1
for r in 1 2 3; do
    REPS=25 ./cohort-bb.sh 1 bb-off-$r
    SOMA_KVM_DEFER_LAUNCH_PAGE_SLOT=1 REPS=25 ./cohort-bb.sh 1 bb-on-$r
done
echo BB_DONE
