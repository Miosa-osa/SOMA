//! The checkpoint and volume half of the production facade.

use super::LocalFacade;
use crate::facade::{
    OwnedVolume, SandboxStorage, VolumeBinding, VolumeBody, VolumeFailure, VolumeOperation,
    VolumeReport,
};
use crate::owners::Owners;

impl SandboxStorage for LocalFacade {
    fn owners(&self) -> Option<&Owners> {
        Some(&self.owners)
    }

    fn checkpoint(
        &mut self,
        instance: &soma::InstanceId,
        operation: &soma::OperationId,
    ) -> Result<soma_local::CheckpointReceipt, soma_local::CheckpointFailure> {
        self.runtime.checkpoint_machine(instance, operation)
    }

    fn discard_checkpoint(
        &mut self,
        instance: &soma::InstanceId,
        checkpoint: &soma::OperationId,
        state_digest: &soma::GenerationId,
    ) -> Result<(), soma_local::CheckpointFailure> {
        self.runtime
            .discard_checkpoint(instance, checkpoint, state_digest)
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn open_checkpoint_export(
        &mut self,
        owner: &soma::MachineName,
        receipt: &soma_local::CheckpointReceipt,
    ) -> Result<soma_local::CheckpointExport, soma_local::CheckpointFailure> {
        self.runtime.open_checkpoint_export(owner, receipt)
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn commit_checkpoint_import(
        &mut self,
        owner: &soma::MachineName,
        receipt: &soma_local::CheckpointReceipt,
        stage: &soma_local::ImportStage,
        absent: bool,
    ) -> Result<(), soma_local::CheckpointFailure> {
        self.runtime
            .commit_checkpoint_import(owner, receipt, stage, absent)
    }

    /// The volume store lives in the host daemon, so this forwards to the control socket and
    /// reports the missing capability when no socket is configured.
    fn volume_binding(&mut self, instance: &str) -> Result<VolumeBinding, VolumeFailure> {
        self.volumes.binding(instance)
    }

    fn volume_owned(&mut self, instance: &str) -> Result<Option<OwnedVolume>, VolumeFailure> {
        self.volumes.owned(instance)
    }

    fn volume(
        &mut self,
        operation: VolumeOperation,
        body: &VolumeBody,
    ) -> Result<VolumeReport, VolumeFailure> {
        if matches!(operation, VolumeOperation::Attach | VolumeOperation::Detach)
            && self.volumes.configured()
            && let Some(report) = self.live_workspace(operation, body)?
        {
            return Ok(report);
        }
        self.volumes.call(operation, body)
    }
}
