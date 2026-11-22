use super::LocalRuntime;

impl LocalRuntime {
    /// Captures a running hosted Machine through its owning process.
    ///
    /// Repeating the same operation id returns the existing durable receipt.
    ///
    /// # Errors
    ///
    /// Returns the owner, capture, or publication failure.
    pub fn checkpoint_machine(
        &self,
        instance: &soma::InstanceId,
        operation: &soma::OperationId,
    ) -> Result<crate::CheckpointReceipt, crate::CheckpointFailure> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let directory = self
                .host_directory
                .as_deref()
                .ok_or(crate::CheckpointFailure::Unsupported)?;
            crate::backend::checkpoint_machine(directory, instance, operation)
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = (instance, operation);
            Err(crate::CheckpointFailure::Unsupported)
        }
    }

    /// Opens the encrypted contents of one authorized capture for an off-host backup.
    ///
    /// # Errors
    ///
    /// Refuses a missing or different owner, an active restore, an unencrypted checkpoint
    /// store, or any mismatch against the full checkpoint receipt supplied by the caller.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub fn open_checkpoint_export(
        &mut self,
        owner: &soma::MachineName,
        expected: &crate::CheckpointReceipt,
    ) -> Result<crate::CheckpointExport, crate::CheckpointFailure> {
        let _lease = self.authorize_checkpoint(owner, expected)?;
        crate::backend::open_export(expected)
    }

    /// Atomically installs a verified backup directory for an existing authorized source.
    ///
    /// # Errors
    ///
    /// Refuses a missing owner, a concurrent restore, an unencrypted stage, or invalid
    /// artifacts. The first successful publication consumes the staging path.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub fn import_checkpoint(
        &mut self,
        owner: &soma::MachineName,
        expected: &crate::CheckpointReceipt,
        stage: &std::path::Path,
    ) -> Result<(), crate::CheckpointFailure> {
        let _lease = self.authorize_checkpoint(owner, expected)?;
        crate::backend::install_import(stage, expected)
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn authorize_checkpoint(
        &mut self,
        owner: &soma::MachineName,
        expected: &crate::CheckpointReceipt,
    ) -> Result<std::fs::File, crate::CheckpointFailure> {
        use crate::CheckpointFailure;

        if self.host_directory.is_none() {
            return Err(CheckpointFailure::Unsupported);
        }
        let lease = crate::file_store::restore_lease(&self.state_root, &expected.instance_id)
            .map_err(|_| CheckpointFailure::Unconfirmed)?;
        let recorded = self
            .engine
            .restore_owner(&expected.instance_id)
            .map_err(|_| CheckpointFailure::Unconfirmed)?;
        if recorded.as_ref() != Some(owner) {
            return Err(CheckpointFailure::NotFound);
        }
        Ok(lease)
    }

    /// Releases a published checkpoint by its exact sandbox, operation and state digest.
    ///
    /// # Errors
    ///
    /// Refuses unconfigured storage, an active restore, a mismatched receipt, or an
    /// unconfirmed storage operation.
    pub fn discard_checkpoint(
        &self,
        instance: &soma::InstanceId,
        checkpoint: &soma::OperationId,
        state_digest: &soma::GenerationId,
    ) -> Result<(), crate::CheckpointFailure> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let state_root = self
                .host_directory
                .as_deref()
                .and_then(std::path::Path::parent)
                .ok_or(crate::CheckpointFailure::Unsupported)?;
            let _lease = crate::file_store::restore_lease(state_root, instance)
                .map_err(|_| crate::CheckpointFailure::Unconfirmed)?;
            crate::backend::discard_checkpoint(instance, checkpoint, state_digest)
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = (instance, checkpoint, state_digest);
            Err(crate::CheckpointFailure::Unsupported)
        }
    }
}
