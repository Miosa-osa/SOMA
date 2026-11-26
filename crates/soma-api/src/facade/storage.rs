//! Checkpoints and durable data volumes: the storage half of the facade the handlers drive.

use super::{OwnedVolume, VolumeBinding, VolumeBody, VolumeFailure, VolumeOperation, VolumeReport};

/// Checkpoint publication and the host daemon's durable data volumes.
///
/// Every [`super::SandboxFacade`] is one of these. A backend with no host control socket keeps
/// the defaults, which report the missing capability rather than an invented answer.
pub trait SandboxStorage {
    /// The tenant owner records this facade's sandboxes and volumes are answered by.
    ///
    /// The default keeps none, and every tenant is then answered alike.
    fn owners(&self) -> Option<&crate::owners::Owners> {
        None
    }

    /// Captures a running sandbox for the supplied operation identity.
    ///
    /// # Errors
    ///
    /// Returns the ownership, capture, or publication failure.
    fn checkpoint(
        &mut self,
        _instance: &soma::InstanceId,
        _operation: &soma::OperationId,
    ) -> Result<soma_local::CheckpointReceipt, soma_local::CheckpointFailure> {
        Err(soma_local::CheckpointFailure::Unsupported)
    }

    /// Discards an exact published checkpoint after its consumer is finished.
    ///
    /// # Errors
    ///
    /// Returns a refusal if the durable publication cannot be safely removed.
    fn discard_checkpoint(
        &mut self,
        _instance: &soma::InstanceId,
        _checkpoint: &soma::OperationId,
        _state_digest: &soma::GenerationId,
    ) -> Result<(), soma_local::CheckpointFailure> {
        Err(soma_local::CheckpointFailure::Unsupported)
    }

    /// Opens a published capture for an off-host backup, for its authorized owner (ADR 0064).
    ///
    /// # Errors
    ///
    /// Returns the owner, receipt, or storage refusal.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn open_checkpoint_export(
        &mut self,
        _owner: &soma::MachineName,
        _receipt: &soma_local::CheckpointReceipt,
    ) -> Result<soma_local::CheckpointExport, soma_local::CheckpointFailure> {
        Err(soma_local::CheckpointFailure::Unsupported)
    }

    /// Verifies and publishes a capture received from off-host storage (ADR 0065, ADR 0066).
    ///
    /// # Errors
    ///
    /// Returns the import's refusal; the stage is kept for a retry or an abort.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn commit_checkpoint_import(
        &mut self,
        _owner: &soma::MachineName,
        _receipt: &soma_local::CheckpointReceipt,
        _stage: &soma_local::ImportStage,
        _absent: bool,
    ) -> Result<(), soma_local::CheckpointFailure> {
        Err(soma_local::CheckpointFailure::Unsupported)
    }

    /// Performs one durable data volume operation through the host daemon.
    ///
    /// The volume store lives inside `soma-hostd` (ADR 0053 Step A), so a backend with no host
    /// control socket configured reports the missing capability rather than an invented answer.
    ///
    /// # Errors
    ///
    /// Returns the typed volume failure: the missing capability when no socket is configured,
    /// the daemon's stable refusal, or a transport failure.
    fn volume(
        &mut self,
        _operation: VolumeOperation,
        _body: &VolumeBody,
    ) -> Result<VolumeReport, VolumeFailure> {
        Err(VolumeFailure::Unconfigured)
    }

    /// Reads which volume one Instance owns, for a stop that must detach it.
    ///
    /// The default answers that this host serves no volumes, which is what a service with no
    /// host control socket has to say rather than a refusal.
    ///
    /// # Errors
    ///
    /// Returns the typed volume failure the host daemon refused with.
    fn volume_owned(&mut self, _instance: &str) -> Result<Option<OwnedVolume>, VolumeFailure> {
        Ok(None)
    }

    /// Opens the volume attached to one Instance, for the launch that binds a machine to it.
    ///
    /// The default reports the missing capability, because a service with no host control socket
    /// serves no volume and a launch that needs one must be refused rather than served without.
    ///
    /// # Errors
    ///
    /// Returns the typed volume failure the host daemon refused with.
    fn volume_binding(&mut self, _instance: &str) -> Result<VolumeBinding, VolumeFailure> {
        Err(VolumeFailure::Unconfigured)
    }
}
