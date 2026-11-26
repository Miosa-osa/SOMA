#!/bin/sh
# Shuts this SOMA instance down or restarts it. PID 1 is the SOMA guest agent, which takes the init
# shutdown signals (SIGUSR2 power off, SIGTERM restart, SIGUSR1 halt) and ends the machine; the
# platform starts it again when it should be running.
set -eu
case "${1:-}" in
  poweroff|shutdown) signal=USR2 ;;
  reboot|restart) signal=TERM ;;
  halt) signal=USR1 ;;
  *) echo "usage: miosa-power poweroff|reboot|halt" >&2; exit 2 ;;
esac
if [ "$(id -u)" -ne 0 ]; then
  exec sudo -n /usr/local/sbin/miosa-power "$1"
fi
sync
kill "-$signal" 1
