use crate::BackendKind;

mod cleanup;
mod execution;
mod failure;
mod inspection;
mod launch;
mod resolution;

pub use cleanup::{CleanupObservation, CleanupReason, CleanupRequest, CleanupTimes};
pub use execution::{CommandObservation, CommandTimes, ExecutionRequest};
pub use failure::{BackendFailure, BackendFailureKind};
pub(crate) use inspection::InspectionObservationParts;
pub use inspection::{InspectionObservation, InspectionRequest};
pub(crate) use launch::LaunchObservationParts;
pub use launch::{LaunchObservation, LaunchRequest, LaunchTimes};
pub use resolution::{ResolutionObservation, ResolutionRequest};

/// Supplies capability-gated execution observations to the portable facade.
///
/// Implementing this trait does not certify a backend and does not turn its observations into
/// cryptographic or hardware attestation.
/// Receipts produced from this seam remain basic backend-reported evidence.
pub trait Backend: Send {
    type PreparedWorkload: Send;

    fn kind(&self) -> BackendKind;

    /// Resolves a mutable OCI reference to an exact workload identity.
    ///
    /// # Errors
    ///
    /// Returns a typed backend failure when resolution cannot produce trustworthy evidence.
    fn resolve(
        &mut self,
        request: ResolutionRequest<'_>,
    ) -> Result<ResolutionObservation<Self::PreparedWorkload>, BackendFailure>;

    /// Launches an admitted workload and reports the effective isolation and shape.
    ///
    /// # Errors
    ///
    /// Returns a typed backend failure when launch or observation fails.
    fn launch(
        &mut self,
        request: LaunchRequest<'_, Self::PreparedWorkload>,
    ) -> Result<LaunchObservation, BackendFailure>;

    /// Executes one bounded direct command in an exact Instance.
    ///
    /// # Errors
    ///
    /// Returns a typed backend failure when command execution cannot be observed.
    fn execute(
        &mut self,
        request: ExecutionRequest<'_>,
    ) -> Result<CommandObservation, BackendFailure>;

    /// Inspects one exact Instance through the bounded portable state model.
    ///
    /// # Errors
    ///
    /// Returns a typed backend failure when the Instance cannot be inspected exactly.
    fn inspect(
        &mut self,
        request: InspectionRequest<'_>,
    ) -> Result<InspectionObservation, BackendFailure>;

    /// Releases resources owned for one exact Instance.
    ///
    /// # Errors
    ///
    /// Returns a typed backend failure when cleanup cannot produce terminal evidence.
    fn cleanup(
        &mut self,
        request: CleanupRequest<'_>,
    ) -> Result<CleanupObservation, BackendFailure>;
}
