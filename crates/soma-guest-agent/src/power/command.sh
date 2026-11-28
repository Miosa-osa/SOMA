#!/bin/sh
# poweroff, reboot, halt and shutdown on a SOMA instance, which has no systemd: each asks PID 1.
case "$(basename "$0")" in
  reboot) exec /usr/local/sbin/miosa-power reboot ;;
  halt) exec /usr/local/sbin/miosa-power halt ;;
  shutdown)
    for argument in "$@"; do
      case "$argument" in -r|--reboot) exec /usr/local/sbin/miosa-power reboot ;; esac
    done
    exec /usr/local/sbin/miosa-power poweroff ;;
  *) exec /usr/local/sbin/miosa-power poweroff ;;
esac
