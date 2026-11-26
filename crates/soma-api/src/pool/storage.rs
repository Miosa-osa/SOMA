//! A lease forwards checkpoint and volume calls to the facade it holds.

use std::ops::DerefMut as _;

use super::FacadeLease;
use crate::facade::{SandboxFacade, SandboxStorage};

impl<F: SandboxFacade> SandboxStorage for FacadeLease<F> {
    fn owners(&self) -> Option<&crate::owners::Owners> {
        (**self).owners()
    }

    fn checkpoint(
        &mut self,
        instance: &soma::InstanceId,
        operation: &soma::OperationId,
    ) -> Result<soma_local::CheckpointReceipt, soma_local::CheckpointFailure> {
        self.deref_mut().checkpoint(instance, operation)
    }

    fn discard_checkpoint(
        &mut self,
        instance: &soma::InstanceId,
        checkpoint: &soma::OperationId,
        state_digest: &soma::GenerationId,
    ) -> Result<(), soma_local::CheckpointFailure> {
        self.deref_mut()
            .discard_checkpoint(instance, checkpoint, state_digest)
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn open_checkpoint_export(
        &mut self,
        owner: &soma::MachineName,
        receipt: &soma_local::CheckpointReceipt,
    ) -> Result<soma_local::CheckpointExport, soma_local::CheckpointFailure> {
        self.deref_mut().open_checkpoint_export(owner, receipt)
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn commit_checkpoint_import(
        &mut self,
        owner: &soma::MachineName,
        receipt: &soma_local::CheckpointReceipt,
        stage: &soma_local::ImportStage,
        absent: bool,
    ) -> Result<(), soma_local::CheckpointFailure> {
        self.deref_mut()
            .commit_checkpoint_import(owner, receipt, stage, absent)
    }

    fn volume(
        &mut self,
        operation: crate::volumes::VolumeOperation,
        body: &crate::volumes::VolumeBody,
    ) -> Result<crate::volumes::VolumeReport, crate::volumes::VolumeFailure> {
        self.deref_mut().volume(operation, body)
    }

    fn volume_owned(
        &mut self,
        instance: &str,
    ) -> Result<Option<crate::volumes::OwnedVolume>, crate::volumes::VolumeFailure> {
        self.deref_mut().volume_owned(instance)
    }

    fn volume_binding(
        &mut self,
        instance: &str,
    ) -> Result<crate::volumes::VolumeBinding, crate::volumes::VolumeFailure> {
        self.deref_mut().volume_binding(instance)
    }
}
