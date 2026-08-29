//! Restore ordering contract: the twelve-step machine-contract order and its unwind rule.
//!
//! This module has no KVM or device effect.
//! The later live restore slice drives [`RestoreSequence`] so that memory is mapped only
//! after compatibility passes, eventfds and irqfds exist before pending interrupt state is
//! armed, fresh authority replaces every captured authority, and the vCPU resumes only after
//! every state constructor succeeded.

use std::{error::Error, fmt};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreStep {
    /// Constant-size manifest identity and compatibility metadata (`compatibility::check`).
    ValidateManifest,
    CreateVm,
    /// `MAP_PRIVATE | MAP_NORESERVE`, no eager copy or population.
    MapMemoryPrivately,
    RegisterMemorySlots,
    /// In-kernel irqchip, fixed GSI routes, and device objects with fresh backends.
    RecreateIrqchipAndDevices,
    CreateVcpu,
    RestoreCpuidAndMsrs,
    /// Special, general, FPU, XCR, XSAVE, LAPIC, MP, event, and optional nested state.
    RestoreVcpuState,
    /// Queues, ioeventfds and irqfds, interrupt-controller state, optional PIT, KVM clock.
    RestoreDeviceAndInterruptState,
    /// Fresh private disk, TAP, vsock endpoint, entropy, and the separate launch page.
    AttachFreshAuthority,
    ResumeVcpu,
    AuthenticatedRepairAndReadiness,
}

impl RestoreStep {
    pub const ORDER: [Self; 12] = [
        Self::ValidateManifest,
        Self::CreateVm,
        Self::MapMemoryPrivately,
        Self::RegisterMemorySlots,
        Self::RecreateIrqchipAndDevices,
        Self::CreateVcpu,
        Self::RestoreCpuidAndMsrs,
        Self::RestoreVcpuState,
        Self::RestoreDeviceAndInterruptState,
        Self::AttachFreshAuthority,
        Self::ResumeVcpu,
        Self::AuthenticatedRepairAndReadiness,
    ];

    /// Steps that acquire owned host or KVM resources and therefore need unwinding.
    #[must_use]
    pub const fn owns_resources(self) -> bool {
        matches!(
            self,
            Self::CreateVm
                | Self::MapMemoryPrivately
                | Self::RegisterMemorySlots
                | Self::RecreateIrqchipAndDevices
                | Self::CreateVcpu
                | Self::AttachFreshAuthority
                | Self::ResumeVcpu
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreOrderError {
    StepOutOfOrder {
        expected: RestoreStep,
        attempted: RestoreStep,
    },
    AlreadyComplete,
    AlreadyFailed(RestoreStep),
}

impl fmt::Display for RestoreOrderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "restore ordering violation: {self:?}")
    }
}

impl Error for RestoreOrderError {}

/// Typed progress through [`RestoreStep::ORDER`].
///
/// A failure freezes the sequence and yields the reverse-order unwind list of every
/// resource-owning step that completed, so cleanup releases in reverse ownership order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestoreSequence {
    completed: usize,
    failed_at: Option<RestoreStep>,
}

impl Default for RestoreSequence {
    fn default() -> Self {
        Self::start()
    }
}

impl RestoreSequence {
    #[must_use]
    pub const fn start() -> Self {
        Self {
            completed: 0,
            failed_at: None,
        }
    }

    #[must_use]
    pub fn next_step(&self) -> Option<RestoreStep> {
        RestoreStep::ORDER.get(self.completed).copied()
    }

    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.failed_at.is_none() && self.next_step().is_none()
    }

    #[must_use]
    pub const fn failed_at(&self) -> Option<RestoreStep> {
        self.failed_at
    }

    /// Records one completed step.
    ///
    /// # Errors
    ///
    /// Returns [`RestoreOrderError::StepOutOfOrder`], [`RestoreOrderError::AlreadyComplete`],
    /// or [`RestoreOrderError::AlreadyFailed`].
    pub fn complete(&mut self, step: RestoreStep) -> Result<(), RestoreOrderError> {
        if let Some(failed) = self.failed_at {
            return Err(RestoreOrderError::AlreadyFailed(failed));
        }
        let expected = self.next_step().ok_or(RestoreOrderError::AlreadyComplete)?;
        if step != expected {
            return Err(RestoreOrderError::StepOutOfOrder {
                expected,
                attempted: step,
            });
        }
        self.completed += 1;
        Ok(())
    }

    /// Records a failure at the next step and returns the unwind order.
    ///
    /// # Errors
    ///
    /// Returns [`RestoreOrderError::AlreadyComplete`] or [`RestoreOrderError::AlreadyFailed`].
    pub fn fail(&mut self) -> Result<Vec<RestoreStep>, RestoreOrderError> {
        if let Some(failed) = self.failed_at {
            return Err(RestoreOrderError::AlreadyFailed(failed));
        }
        let step = self.next_step().ok_or(RestoreOrderError::AlreadyComplete)?;
        self.failed_at = Some(step);
        Ok(self.unwind_order())
    }

    /// Resource-owning completed steps in reverse order.
    #[must_use]
    pub fn unwind_order(&self) -> Vec<RestoreStep> {
        RestoreStep::ORDER[..self.completed]
            .iter()
            .rev()
            .copied()
            .filter(|step| step.owns_resources())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{RestoreOrderError, RestoreSequence, RestoreStep};

    #[test]
    fn steps_complete_in_the_documented_order_only() {
        let mut sequence = RestoreSequence::start();
        assert_eq!(
            sequence.complete(RestoreStep::MapMemoryPrivately),
            Err(RestoreOrderError::StepOutOfOrder {
                expected: RestoreStep::ValidateManifest,
                attempted: RestoreStep::MapMemoryPrivately
            })
        );
        for step in RestoreStep::ORDER {
            sequence.complete(step).unwrap();
        }
        assert!(sequence.is_ready());
        assert_eq!(
            sequence.complete(RestoreStep::ResumeVcpu),
            Err(RestoreOrderError::AlreadyComplete)
        );
        assert_eq!(sequence.fail(), Err(RestoreOrderError::AlreadyComplete));
        assert_eq!(RestoreSequence::default(), RestoreSequence::start());
    }

    #[test]
    fn failure_yields_reverse_ownership_unwind_and_freezes_the_sequence() {
        let mut sequence = RestoreSequence::start();
        for step in &RestoreStep::ORDER[..7] {
            sequence.complete(*step).unwrap();
        }
        let unwind = sequence.fail().unwrap();
        assert_eq!(
            unwind,
            vec![
                RestoreStep::CreateVcpu,
                RestoreStep::RecreateIrqchipAndDevices,
                RestoreStep::RegisterMemorySlots,
                RestoreStep::MapMemoryPrivately,
                RestoreStep::CreateVm,
            ]
        );
        assert_eq!(sequence.failed_at(), Some(RestoreStep::RestoreVcpuState));
        assert!(!sequence.is_ready());
        assert_eq!(
            sequence.complete(RestoreStep::RestoreVcpuState),
            Err(RestoreOrderError::AlreadyFailed(
                RestoreStep::RestoreVcpuState
            ))
        );
        assert_eq!(
            sequence.fail(),
            Err(RestoreOrderError::AlreadyFailed(
                RestoreStep::RestoreVcpuState
            ))
        );
    }
}
