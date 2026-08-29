//! Launch-page operations while the guest runs: observing consumption and retiring the slot.

use std::{
    sync::{PoisonError, atomic::Ordering},
    thread,
    time::{Duration, Instant},
};

use super::{Milestone, SandboxMachine};
use crate::x86_64::{
    error::{MachineError, MachineErrorKind, Phase},
    launch_page::LAUNCH_PAGE_SIZE,
};

const CONSUME_POLL: Duration = Duration::from_millis(1);

impl SandboxMachine {
    /// Publishes one launch page before the guest runs.
    ///
    /// # Errors
    ///
    /// Fails when the slot was already retired.
    pub fn write_launch_page(&self, page: &[u8; LAUNCH_PAGE_SIZE]) -> Result<(), MachineError> {
        let mut slot = self
            .launch_page
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let slot = slot
            .as_mut()
            .ok_or_else(|| MachineError::invalid(Phase::LaunchPage, "launch page slot retired"))?;
        slot.write(page)?;
        self.mark(Milestone::LaunchPageWritten);
        Ok(())
    }

    /// Polls until the guest overwrote the launch page domain `domain`, or the deadline.
    ///
    /// # Errors
    ///
    /// Returns a timeout, or a failure when the vCPU stopped before consuming the page.
    pub fn wait_launch_page_consumed(
        &self,
        domain: &[u8],
        deadline: Instant,
    ) -> Result<(), MachineError> {
        loop {
            let present = {
                let slot = self
                    .launch_page
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                slot.as_ref().is_some_and(|slot| slot.starts_with(domain))
            };
            if !present {
                self.mark(Milestone::LaunchPageConsumed);
                return Ok(());
            }
            if self.finished.load(Ordering::Acquire) {
                return Err(MachineError::invalid(
                    Phase::LaunchPage,
                    "guest stopped before consuming the launch page",
                ));
            }
            if Instant::now() >= deadline {
                return Err(MachineError::new(
                    Phase::LaunchPage,
                    MachineErrorKind::Timeout,
                ));
            }
            thread::sleep(CONSUME_POLL);
        }
    }

    /// Verifies the guest erased the page, removes the slot, and unmaps it.
    ///
    /// # Errors
    ///
    /// Returns `LaunchPageNotErased` when material remained; the slot is removed regardless.
    pub fn retire_launch_page(&self) -> Result<(), MachineError> {
        let slot = self
            .launch_page
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or_else(|| {
                MachineError::invalid(Phase::LaunchPage, "launch page already retired")
            })?;
        slot.retire(&self.machine.vm)?;
        self.mark(Milestone::LaunchPageRetired);
        Ok(())
    }

    /// Whether the launch page slot has already been retired.
    pub(super) fn launch_page_retired(&self) -> bool {
        self.launch_page
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none()
    }
}
