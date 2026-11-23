#!/bin/sh
#
# The desktop Generation's capture warm plan (SOMA_CAPTURE_WARM in scripts/desktop-generation.sh).
#
# Run once by the guest agent at the capture point, as nobody, on a read-only root, before any
# Instance exists. It only reads: every program the session starts and every shared library they
# load, plus the fonts and icon caches they open, so those pages sit in the guest page cache that
# the snapshot records. Every desktop restored from the Generation then starts its session from
# memory instead of reading tens of megabytes from disk (item 2 of the 2026-11-23 bloat list).
# Nothing it does can leave a file, a process or a secret behind: the agent kills it afterwards.

PROGRAMS="/usr/bin/Xvfb /usr/bin/openbox /usr/bin/pcmanfm /usr/bin/tint2 /usr/bin/plank /usr/bin/picom
/usr/bin/dbus-daemon /usr/libexec/at-spi-bus-launcher /usr/libexec/at-spi2-registryd
/usr/local/bin/voxl /usr/local/bin/cua-driver /usr/bin/xprop /usr/bin/xwininfo /usr/bin/xset"

for p in $PROGRAMS; do
    [ -f "$p" ] || continue
    cat "$p" >/dev/null 2>&1
    # The libraries the dynamic linker would map for it.
    ldd "$p" 2>/dev/null | awk '/=> \//{print $3} /^\t\//{print $1}'
done | sort -u | while read -r lib; do cat "$lib" >/dev/null 2>&1; done

# What the session reads on its first frame: the font configuration and caches, the icon theme
# caches, the wallpaper.
cat /etc/fonts/fonts.conf /var/cache/fontconfig/* /usr/share/icons/*/icon-theme.cache \
    /usr/share/backgrounds/miosa/miosa-aurora.jpg >/dev/null 2>&1
exit 0
