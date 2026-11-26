use std::time::Duration;

use soma::BackendFailureKind;
use soma_vmm::control::{Outcome, Request};
use soma_vmm::sandbox::{MemoryError, MemoryLevel};

use super::Jailed;

/// How long one memory exchange may take: the worker answers from the device at once.
const MEMORY_CEILING: Duration = Duration::from_secs(10);

impl Jailed {
    /// Asks the worker's memory device to plug `requested` bytes in total, or with `None` reads
    /// its level.
    ///
    /// The inner result is the device's answer. A worker that predates the request refuses its
    /// word, which reads as a machine with no device rather than a broken one.
    pub(in crate::backend::kvm) fn memory(
        &mut self,
        requested: Option<u64>,
    ) -> Result<Result<MemoryLevel, MemoryError>, BackendFailureKind> {
        if !self.is_usable() {
            return Err(BackendFailureKind::Unavailable);
        }
        match self
            .control
            .ask(&Request::Memory(self.instance, requested), MEMORY_CEILING)
        {
            Ok(Outcome::Memory(answer)) => Ok(answer),
            Ok(Outcome::Rejected(_)) => Ok(Err(MemoryError::NoDevice)),
            _ => Err(self.poison(BackendFailureKind::GuestFailure)),
        }
    }
}
