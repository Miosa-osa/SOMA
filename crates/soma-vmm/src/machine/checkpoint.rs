use std::time::Duration;

use soma_kvm::snapshot::{Digest, MemoryAttestation};

use super::{Machine, State};
use crate::{
    Failure, InstanceId, Launch, Ready,
    sandbox::{CheckpointJob, CheckpointJobError},
};

impl Machine {
    /// Launches the authorized checkpoint on a worker with no existing Instance.
    ///
    /// `memory` is the trusted broker's attestation of the checkpoint memory file; the file is
    /// re-read unless it describes the mapped descriptor exactly.
    ///
    /// # Errors
    ///
    /// Refuses a used worker and any invalid checkpoint or unauthenticated recovery.
    pub fn recover(
        &mut self,
        launch: Launch,
        digest: Digest,
        saved_overlay: Option<std::fs::File>,
        memory: Option<MemoryAttestation>,
    ) -> Result<Ready, Failure> {
        self.launch_checkpoint(launch, digest, saved_overlay, None, memory)
    }

    /// Launches an authorized checkpoint in a fresh Instance with separate guest authority.
    /// # Errors
    /// Refuses a reused source identity or any invalid checkpoint.
    pub fn fork(
        &mut self,
        launch: Launch,
        digest: Digest,
        saved_overlay: Option<std::fs::File>,
        source: InstanceId,
        memory: Option<MemoryAttestation>,
    ) -> Result<Ready, Failure> {
        if source == launch.instance_id() {
            return Err(super::fault::lifecycle_failure());
        }
        self.launch_checkpoint(launch, digest, saved_overlay, Some(source), memory)
    }

    fn launch_checkpoint(
        &mut self,
        launch: Launch,
        digest: Digest,
        saved_overlay: Option<std::fs::File>,
        fork_from: Option<InstanceId>,
        memory: Option<MemoryAttestation>,
    ) -> Result<Ready, Failure> {
        if self.state != State::New
            || saved_overlay.is_some() != launch.generation().devices().writable_disk()
        {
            return Err(super::fault::lifecycle_failure());
        }
        self.platform
            .configure_recovery(digest, saved_overlay, fork_from, memory)
            .map_err(|_| super::fault::lifecycle_failure())?;
        self.launch(launch)
    }

    /// Captures the running Instance into broker-owned descriptors.
    ///
    /// # Errors
    ///
    /// Returns a refusal for a different Instance or invalid input, or reports when a live
    /// worker can no longer establish that the source is running.
    pub fn checkpoint(
        &mut self,
        job: CheckpointJob,
        deadline: Duration,
    ) -> Result<(InstanceId, Digest), CheckpointJobError> {
        let instance = InstanceId::new(job.source.instance_id)
            .map_err(|_| CheckpointJobError::CaptureRefused)?;
        if self.state != State::Ready || self.instance_id != Some(instance) {
            return Err(CheckpointJobError::CaptureRefused);
        }
        match self.platform.checkpoint(job, deadline) {
            Ok(digest) => Ok((instance, digest)),
            Err(CheckpointJobError::SourceUnconfirmed) => {
                self.state = State::Failed;
                self.guest = None;
                Err(CheckpointJobError::SourceUnconfirmed)
            }
            Err(error) => Err(error),
        }
    }
}
