use crate::{BackendKind, InstanceId};

mod cleanup;
mod execution;
mod failure;
mod file;
mod inspection;
mod launch;
mod liveness;
mod pty;
mod resolution;

pub use cleanup::{CleanupObservation, CleanupReason, CleanupRequest, CleanupTimes};
pub use execution::{CommandObservation, CommandTimes, ExecutionRequest};
pub use failure::{BackendFailure, BackendFailureKind};
pub use file::{
    FileAnswer, FileEntry, FileKind, FileObservation, FileOperation, FileRefusal, FileRequest,
    MAX_FILE_BYTES, MAX_GUEST_PATH_BYTES, PathRejected, check_guest_path,
};
pub use liveness::SandboxLiveness;
pub use pty::{
    MAX_PTY_CHUNK_BYTES, MAX_PTY_COLUMNS, MAX_PTY_ROWS, MAX_PTY_WAIT_MILLIS, PtyAnswer,
    PtyObservation, PtyOperation, PtyRefusal, PtyRejected, PtyRequest,
};

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

    /// Performs one bounded filesystem operation inside an exact Instance.
    ///
    /// A backend that holds no machine a later call could address cannot serve this, and says so
    /// with [`BackendFailureKind::Unsupported`] rather than inventing an answer.
    ///
    /// # Errors
    ///
    /// Returns a typed backend failure when the operation cannot be performed or observed. A
    /// filesystem cause the guest reported is not a failure: it is carried back as
    /// [`FileAnswer::Refused`], because the operation was performed and the guest declined it.
    fn file(&mut self, request: FileRequest<'_>) -> Result<FileObservation, BackendFailure>;

    /// Performs one bounded terminal operation inside an exact Instance.
    ///
    /// A backend that holds no machine a later call could address cannot serve this, and says so
    /// with [`BackendFailureKind::Unsupported`] rather than inventing an answer.
    ///
    /// # Errors
    ///
    /// Returns a typed backend failure when the operation cannot be performed or observed. A
    /// cause the guest reported is not a failure: it is carried back as [`PtyAnswer::Refused`],
    /// because the operation was performed and the guest declined it.
    fn pty(&mut self, request: PtyRequest<'_>) -> Result<PtyObservation, BackendFailure>;

    /// Reports whether anything is still serving one exact Instance.
    ///
    /// This is a probe rather than a lifecycle operation: it runs nothing in the guest, changes
    /// nothing, and produces no evidence. A backend that cannot answer says
    /// [`SandboxLiveness::Unknown`] rather than guessing, because a guess here would be reported
    /// to a caller as the state of a sandbox it is deciding whether to use.
    fn liveness(&mut self, instance_id: &InstanceId) -> SandboxLiveness;

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
