#!/bin/sh
#
# /usr/local/bin/cua-driver: the pinned driver with telemetry and the update check off.
#
# The driver reports usage to its vendor and checks for new releases by default. Both are
# switched off here, for every caller, in the environment the driver reads first; the image also
# writes the persisted preference (~/.cua-driver/config.json) for the desktop user and root, so a
# caller that bypasses this wrapper still gets both off. Nothing in a guest calls the vendor.
export CUA_DRIVER_RS_TELEMETRY_ENABLED=0
export CUA_TELEMETRY_ENABLED=0
export CUA_DRIVER_RS_UPDATE_CHECK=0
exec /opt/cua-driver/cua-driver "$@"
