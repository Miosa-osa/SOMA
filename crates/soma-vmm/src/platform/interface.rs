use crate::{Execute, Launch, Stop};

use super::{
    PlatformExecution, PlatformFailure, PlatformStop, ReadinessFailure, ReadyAuthenticatedGuest,
    RestoreFailure, RestoredMachine,
};

pub(crate) trait Platform: Send {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn configure_recovery(
        &mut self,
        _digest: soma_kvm::snapshot::Digest,
        _saved_overlay: Option<std::fs::File>,
        _fork_from: Option<crate::InstanceId>,
    ) -> Result<(), PlatformFailure> {
        Err(PlatformFailure::new(crate::Recovery::DoNotRetry))
    }

    /// Verifies the exact Generation and restores one private machine as one cohesive operation.
    fn verify_and_restore(&mut self, launch: &Launch) -> Result<RestoredMachine, RestoreFailure>;

    /// Authenticates, repairs, and probes the restored guest through one fused adapter operation.
    fn authenticate_repair_and_ready(
        &mut self,
        launch: &Launch,
        restored: RestoredMachine,
    ) -> Result<ReadyAuthenticatedGuest, ReadinessFailure>;

    fn execute(
        &mut self,
        execute: &Execute,
        guest: &mut ReadyAuthenticatedGuest,
    ) -> Result<PlatformExecution, PlatformFailure>;

    /// Executes a command and reports each output chunk to `sink`.
    ///
    /// The default runs the command to its end and then replays the retained output, so a
    /// platform that cannot stream still answers a streaming request, honestly late. A platform
    /// that can stream overrides this and calls `sink` while the command runs.
    fn execute_streaming(
        &mut self,
        execute: &Execute,
        guest: &mut ReadyAuthenticatedGuest,
        sink: &mut dyn FnMut(crate::control::OutputStream, &[u8]),
    ) -> Result<PlatformExecution, PlatformFailure> {
        let outcome = self.execute(execute, guest)?;
        outcome.replay(sink);
        Ok(outcome)
    }

    /// Sets how much the memory device should plug, or with `None` reads its level.
    ///
    /// The inner result is the device's own answer; the outer one is a session that can no
    /// longer be trusted. A platform with no memory device answers that it has none.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn memory(
        &mut self,
        _requested: Option<u64>,
    ) -> Result<Result<crate::sandbox::MemoryLevel, crate::sandbox::MemoryError>, PlatformFailure>
    {
        Ok(Err(crate::sandbox::MemoryError::NoDevice))
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn file(
        &mut self,
        _operation: soma::FileOperation,
    ) -> Result<soma::FileAnswer, PlatformFailure> {
        Err(PlatformFailure::new(crate::Recovery::DoNotRetry))
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn pty(&mut self, _operation: &soma::PtyOperation) -> Result<soma::PtyAnswer, PlatformFailure> {
        Err(PlatformFailure::new(crate::Recovery::DoNotRetry))
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn checkpoint(
        &mut self,
        _job: crate::sandbox::CheckpointJob,
        _deadline: std::time::Duration,
    ) -> Result<soma_kvm::snapshot::Digest, crate::sandbox::CheckpointJobError> {
        Err(crate::sandbox::CheckpointJobError::CaptureRefused)
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn change_workspace(
        &mut self,
        _change: crate::sandbox::WorkspaceChange,
    ) -> Result<soma_guest::WorkspaceOutcome, PlatformFailure> {
        Ok(soma_guest::WorkspaceOutcome::Unsupported)
    }

    fn pause(&mut self) -> Result<(), PlatformFailure> {
        Err(PlatformFailure::new(crate::Recovery::RepairHost))
    }

    fn resume(&mut self) -> Result<(), PlatformFailure> {
        Err(PlatformFailure::new(crate::Recovery::RepairHost))
    }

    fn stop(
        &mut self,
        stop: &Stop,
        guest: Option<&mut ReadyAuthenticatedGuest>,
    ) -> Result<PlatformStop, PlatformFailure>;

    fn rollback(&mut self, launch: &Launch) -> Result<(), PlatformFailure>;
}
