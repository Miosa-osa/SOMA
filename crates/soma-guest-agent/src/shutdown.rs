//! Authenticated shutdown: refuse new work, kill children, sync, acknowledge, power off.
//!
//! New work is refused structurally because the typestate controller has already left
//! `Ready`, so no request loop can run after this function is entered.

use std::time::{Duration, Instant};

use soma_guest::{ControlIo, GuestControl};

use crate::{console, descendants, pid1};

/// Budget for delivering the single acknowledgement record.
pub const ACK_BUDGET: Duration = Duration::from_secs(5);

/// Terminates every child, flushes filesystems, acknowledges, and powers the machine off.
pub fn perform<I: ControlIo>(control: GuestControl<I>) -> ! {
    descendants::sweep_strays();
    pid1::reap_orphans();
    pid1::sync();
    match control.shutdown_ack(Instant::now() + ACK_BUDGET) {
        Ok(()) => console::report("shutdown acknowledged"),
        Err(error) => console::report(&format!("shutdown acknowledgement failed: {error}")),
    }
    pid1::poweroff()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_acknowledgement_budget_matches_the_host_shutdown_ceiling() {
        assert_eq!(ACK_BUDGET, Duration::from_secs(5));
    }
}
