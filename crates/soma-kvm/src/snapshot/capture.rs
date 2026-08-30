//! Capture ordering contract: quiesce preconditions and the fixed read and publish order.
//!
//! This module has no KVM or device effect.
//! It is the typed schedule the later live capture slice must drive, so ordering mistakes
//! become compile-time or typed-error failures rather than silently unsafe snapshots.

use std::{error::Error, fmt};

/// Conditions the builder must prove, in order, before any state is read.
///
/// Capture happens before any launch material exists, so there is no authenticated session
/// to prove and none to scrub; what the builder proves instead is that the Generation's own
/// pinned guest agent is the code that parked the machine at the disconnected repair point
/// (ADR 0030, the pre-launch snapshot capture point).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuiescePrecondition {
    /// The Generation's pinned guest agent reached its own code and announced itself.
    GenerationAgentBooted,
    /// The agent is blocked in the launch-page wait with no session and no Instance identity.
    RepairPointReached,
    IngressDisabled,
    DeviceWorkDrained,
    OverlayFlushed,
    VcpuPaused,
    QueuesProvenQuiescent,
}

impl QuiescePrecondition {
    pub const ORDER: [Self; 7] = [
        Self::GenerationAgentBooted,
        Self::RepairPointReached,
        Self::IngressDisabled,
        Self::DeviceWorkDrained,
        Self::OverlayFlushed,
        Self::VcpuPaused,
        Self::QueuesProvenQuiescent,
    ];
}

/// Fixed order in which state is read while the vCPU stays joined outside `KVM_RUN`,
/// followed by staging, independent decode, hashing, and atomic-last publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureStep {
    ReadVmState,
    ReadVcpuState,
    ReadIrqchip,
    ReadIrqRouting,
    ReadClock,
    ReadPit,
    ReadDevices,
    WriteStagingObjects,
    IndependentlyDecodeStaging,
    HashThroughRetainedHandles,
    PublishGenerationManifest,
}

impl CaptureStep {
    pub const ORDER: [Self; 11] = [
        Self::ReadVmState,
        Self::ReadVcpuState,
        Self::ReadIrqchip,
        Self::ReadIrqRouting,
        Self::ReadClock,
        Self::ReadPit,
        Self::ReadDevices,
        Self::WriteStagingObjects,
        Self::IndependentlyDecodeStaging,
        Self::HashThroughRetainedHandles,
        Self::PublishGenerationManifest,
    ];

    /// `ReadPit` may be skipped when the certified profile carries no PIT section.
    #[must_use]
    pub const fn is_optional(self) -> bool {
        matches!(self, Self::ReadPit)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureOrderError {
    PreconditionOutOfOrder {
        expected: QuiescePrecondition,
        attempted: QuiescePrecondition,
    },
    PreconditionsIncomplete(QuiescePrecondition),
    StepOutOfOrder {
        expected: CaptureStep,
        attempted: CaptureStep,
    },
    AlreadyComplete,
}

impl fmt::Display for CaptureOrderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "capture ordering violation: {self:?}")
    }
}

impl Error for CaptureOrderError {}

/// Quiesce phase: every precondition must be proven in order before capture may begin.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Quiesce {
    proven: usize,
}

impl Default for Quiesce {
    fn default() -> Self {
        Self::new()
    }
}

impl Quiesce {
    #[must_use]
    pub const fn new() -> Self {
        Self { proven: 0 }
    }

    #[must_use]
    pub fn next_precondition(&self) -> Option<QuiescePrecondition> {
        QuiescePrecondition::ORDER.get(self.proven).copied()
    }

    /// Records one proven precondition.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureOrderError::PreconditionOutOfOrder`] or
    /// [`CaptureOrderError::AlreadyComplete`].
    pub fn prove(&mut self, precondition: QuiescePrecondition) -> Result<(), CaptureOrderError> {
        let expected = self
            .next_precondition()
            .ok_or(CaptureOrderError::AlreadyComplete)?;
        if precondition != expected {
            return Err(CaptureOrderError::PreconditionOutOfOrder {
                expected,
                attempted: precondition,
            });
        }
        self.proven += 1;
        Ok(())
    }

    /// Consumes the proven quiesce and starts the capture sequence.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureOrderError::PreconditionsIncomplete`] naming the first unproven
    /// precondition.
    pub fn begin_capture(self) -> Result<CaptureSequence, CaptureOrderError> {
        match self.next_precondition() {
            Some(missing) => Err(CaptureOrderError::PreconditionsIncomplete(missing)),
            None => Ok(CaptureSequence { completed: 0 }),
        }
    }
}

/// Capture phase: steps complete strictly in [`CaptureStep::ORDER`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureSequence {
    completed: usize,
}

impl CaptureSequence {
    #[must_use]
    pub fn next_step(&self) -> Option<CaptureStep> {
        CaptureStep::ORDER.get(self.completed).copied()
    }

    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.next_step().is_none()
    }

    /// Records one completed step; an optional step may be skipped by completing the
    /// step that follows it.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureOrderError::StepOutOfOrder`] or [`CaptureOrderError::AlreadyComplete`].
    pub fn complete(&mut self, step: CaptureStep) -> Result<(), CaptureOrderError> {
        let expected = self.next_step().ok_or(CaptureOrderError::AlreadyComplete)?;
        if step == expected {
            self.completed += 1;
            return Ok(());
        }
        let skip_optional =
            expected.is_optional() && CaptureStep::ORDER.get(self.completed + 1) == Some(&step);
        if skip_optional {
            self.completed += 2;
            return Ok(());
        }
        Err(CaptureOrderError::StepOutOfOrder {
            expected,
            attempted: step,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{CaptureOrderError, CaptureStep, Quiesce, QuiescePrecondition};

    #[test]
    fn preconditions_must_be_proven_in_order_before_capture() {
        let mut quiesce = Quiesce::new();
        assert_eq!(
            quiesce.begin_capture(),
            Err(CaptureOrderError::PreconditionsIncomplete(
                QuiescePrecondition::GenerationAgentBooted
            ))
        );
        assert_eq!(
            quiesce.prove(QuiescePrecondition::VcpuPaused),
            Err(CaptureOrderError::PreconditionOutOfOrder {
                expected: QuiescePrecondition::GenerationAgentBooted,
                attempted: QuiescePrecondition::VcpuPaused
            })
        );
        for precondition in QuiescePrecondition::ORDER {
            quiesce.prove(precondition).unwrap();
        }
        assert_eq!(
            quiesce.prove(QuiescePrecondition::GenerationAgentBooted),
            Err(CaptureOrderError::AlreadyComplete)
        );
        let mut sequence = quiesce.begin_capture().unwrap();
        assert_eq!(
            sequence.complete(CaptureStep::PublishGenerationManifest),
            Err(CaptureOrderError::StepOutOfOrder {
                expected: CaptureStep::ReadVmState,
                attempted: CaptureStep::PublishGenerationManifest
            })
        );
        for step in CaptureStep::ORDER {
            if step == CaptureStep::ReadPit {
                continue;
            }
            sequence.complete(step).unwrap();
        }
        assert!(sequence.is_complete());
        assert_eq!(
            sequence.complete(CaptureStep::ReadVmState),
            Err(CaptureOrderError::AlreadyComplete)
        );
        assert_eq!(Quiesce::default(), Quiesce::new());
    }
}
