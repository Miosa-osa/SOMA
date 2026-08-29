//! PID 1 duties: never exit, reap orphans, and turn any panic into a controlled poweroff.
//!
//! As init, the kernel ignores every signal with default disposition, so the agent installs no
//! terminal signal handlers and no shell.
//! Orphan reaping is explicit and only runs at points where no executor is waiting on a child,
//! because a background `waitpid(-1)` would steal the executor's exit status.

#![allow(unsafe_code)]

use std::panic;

use crate::console;

const EXIT_NOT_PID1: i32 = 3;

/// Returns whether this process is the init process of the guest.
#[must_use]
pub fn is_pid1() -> bool {
    std::process::id() == 1
}

/// Installs the panic hook that reports a bounded diagnostic and powers the machine off.
pub fn install_panic_hook() {
    panic::set_hook(Box::new(|info| {
        let location = info.location().map_or_else(String::new, |location| {
            format!("{}:{}", location.file(), location.line())
        });
        let reason = info
            .payload()
            .downcast_ref::<&str>()
            .map(|message| (*message).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        console::report(&format!("panic at {location}: {reason}"));
        poweroff();
    }));
}

/// Schedules every dirty filesystem page for write-back.
pub fn sync() {
    // SAFETY: `sync` takes no arguments and only schedules dirty pages for write-back.
    unsafe { libc::sync() };
}

/// Flushes filesystems and stops the machine without ever returning.
///
/// The version 1 machine contract offers no ACPI and no paravirtual power-off, so a
/// `LINUX_REBOOT_CMD_POWER_OFF` request degrades to `halt`, which parks the vCPU inside KVM
/// with interrupts disabled and is invisible to the host. The contract boots with `reboot=k`,
/// so the restart command pulses the keyboard-controller reset line instead, which the VMM
/// observes as the orderly `Reset` exit; the machine is single-use and never actually restarts.
///
/// Outside PID 1 the process exits with a distinct code instead of rebooting the host.
pub fn poweroff() -> ! {
    sync();
    if is_pid1() {
        // SAFETY: `reboot` with `LINUX_REBOOT_CMD_RESTART` takes one integer command and
        // affects only this machine; it is only reached when this process is the guest init.
        unsafe { libc::reboot(libc::LINUX_REBOOT_CMD_RESTART) };
        loop {
            // SAFETY: `pause` has no preconditions and merely blocks until a signal arrives.
            unsafe { libc::pause() };
        }
    }
    std::process::exit(EXIT_NOT_PID1)
}

/// Reaps every already-exited orphan as PID 1 and returns the number reaped.
///
/// Callers must invoke this only while no other child is being awaited elsewhere.
pub fn reap_orphans() -> usize {
    if !is_pid1() {
        return 0;
    }
    let mut reaped = 0;
    loop {
        let mut status = 0;
        // SAFETY: `waitpid` writes one integer into the provided valid local and never blocks
        // with `WNOHANG`; `-1` selects any child, which is safe here because PID 1 owns every
        // orphan and no executor is concurrently waiting.
        let pid = unsafe { libc::waitpid(-1, &raw mut status, libc::WNOHANG) };
        if pid <= 0 {
            return reaped;
        }
        reaped += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_process_is_never_treated_as_init() {
        assert!(!is_pid1());
        assert_eq!(reap_orphans(), 0);
    }
}
