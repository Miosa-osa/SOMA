//! What a worker does when its guest ends the machine from inside.
//!
//! A worker process holds exactly one machine, so once the guest has shut down or restarted, the
//! process has nothing left to hold, and its exit status is how it tells the process that
//! supervises it which of the two the guest asked for. The machine is already finished when the
//! status is chosen. Only a process that opted in exits: a test or an embedder that runs a
//! sandbox inside a longer-lived process keeps running, and its session simply ends.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use soma_kvm::x86_64::GuestEnd;

/// The worker's exit status for a guest that shut down from inside.
pub const GUEST_POWER_OFF_STATUS: i32 = 8;
/// The worker's exit status for a guest that restarted from inside.
pub const GUEST_REBOOT_STATUS: i32 = 9;

static EXIT_ON_GUEST_END: AtomicBool = AtomicBool::new(false);
/// The last end a guest of this process asked for and has not been answered yet.
static OBSERVED: AtomicU8 = AtomicU8::new(0);

/// Makes this process exit, with the status naming the end, when its guest ends the machine.
pub fn exit_process_on_guest_end() {
    EXIT_ON_GUEST_END.store(true, Ordering::Release);
}

/// The exit status naming `end`.
#[must_use]
pub const fn status(end: GuestEnd) -> i32 {
    match end {
        GuestEnd::PowerOff => GUEST_POWER_OFF_STATUS,
        GuestEnd::Reboot => GUEST_REBOOT_STATUS,
    }
}

/// The end a worker's exit status names, when it names one.
#[must_use]
pub const fn from_status(status: i32) -> Option<GuestEnd> {
    match status {
        GUEST_POWER_OFF_STATUS => Some(GuestEnd::PowerOff),
        GUEST_REBOOT_STATUS => Some(GuestEnd::Reboot),
        _ => None,
    }
}

/// The end a guest of this process asked for since it was last taken, if any.
///
/// A process that holds one machine in its own sandbox thread reads this to learn that the
/// guest shut down or restarted, and takes it so the next machine starts unobserved.
#[must_use]
pub fn take_observed() -> Option<GuestEnd> {
    from_status(i32::from(OBSERVED.swap(0, Ordering::AcqRel)))
}

/// Called once the machine the guest ended is finished.
pub(super) fn ended(end: GuestEnd) {
    if EXIT_ON_GUEST_END.load(Ordering::Acquire) {
        std::process::exit(status(end));
    }
    OBSERVED.store(u8::try_from(status(end)).unwrap_or(0), Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_guest_end_has_its_own_status_and_reads_back() {
        for end in [GuestEnd::PowerOff, GuestEnd::Reboot] {
            assert_eq!(from_status(status(end)), Some(end));
        }
        assert_ne!(status(GuestEnd::PowerOff), status(GuestEnd::Reboot));
        assert_eq!(from_status(0), None);
        assert_eq!(from_status(7), None);
    }

    #[test]
    fn an_observed_end_is_taken_once() {
        ended(GuestEnd::Reboot);
        assert_eq!(take_observed(), Some(GuestEnd::Reboot));
        assert_eq!(take_observed(), None);
    }
}
