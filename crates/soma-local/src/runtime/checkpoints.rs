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

    /// Imports the workspace of a verified backup without restoring its CPU or memory.
    ///
    /// The caller authenticates the tenant, fences the original placement, and chooses the
    /// candidate Generation for the destination's cold launch.
    ///
    /// # Errors
    ///
    /// Refuses an unavailable workspace, changed destination, or invalid capture.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub fn import_cold_workspace(
        &self,
        expected: &crate::CheckpointReceipt,
        stage: &std::path::Path,
        target: &crate::ColdWorkspaceTarget,
    ) -> Result<(), crate::CheckpointFailure> {
        let _lease = self.checkpoint_lease(expected)?;
        crate::backend::import_cold_workspace(stage, expected, target)
    }

    /// Installs a verified backup and reserves recovery authority for an absent Instance.
    ///
    /// The caller must authenticate the backup owner and fence the original Instance.
    ///
    /// # Errors
    ///
    /// Refuses conflicting state, unsupported storage, or an unverified checkpoint.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub fn recover_absent_checkpoint(
        &mut self,
        owner: &soma::MachineName,
        expected: &crate::CheckpointReceipt,
        stage: &std::path::Path,
    ) -> Result<(), crate::CheckpointFailure> {
        use crate::CheckpointFailure;
        let lease = self.checkpoint_lease(expected)?;
        let digest = receipt_digest(expected)?;
        // Validate the stage before claiming a fresh machine ID. An interrupted install can
        // already have consumed the stage, so verification also accepts the exact native target.
        let publication = crate::backend::verify_import(stage, expected)?;
        let result = self
            .engine
            .install_recovery_source(
                &expected.instance_id,
                owner,
                &expected.checkpoint_id,
                &expected.generation_id,
                &digest,
                || publication.install(),
            )
            .map_err(|error| match error {
                soma::ManagedFailure::State(soma::ManagedStateError::OperationConflict)
                | soma::ManagedFailure::StateStore(soma::StateStoreFailureKind::Conflict) => {
                    CheckpointFailure::CaptureRefused
                }
                _ => CheckpointFailure::Unconfirmed,
            })?;
        drop(lease);
        result
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn authorize_checkpoint(
        &mut self,
        owner: &soma::MachineName,
        expected: &crate::CheckpointReceipt,
    ) -> Result<std::fs::File, crate::CheckpointFailure> {
        use crate::CheckpointFailure;

        let lease = self.checkpoint_lease(expected)?;
        let recorded = self
            .engine
            .restore_owner(&expected.instance_id)
            .map_err(|_| CheckpointFailure::Unconfirmed)?;
        if recorded.as_ref() != Some(owner) {
            return Err(CheckpointFailure::NotFound);
        }
        let digest = receipt_digest(expected)?;
        if !self
            .engine
            .permits_checkpoint_transfer(
                &expected.instance_id,
                &expected.checkpoint_id,
                &expected.generation_id,
                &digest,
            )
            .map_err(|_| CheckpointFailure::Unconfirmed)?
        {
            return Err(CheckpointFailure::NotFound);
        }
        Ok(lease)
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn checkpoint_lease(
        &self,
        expected: &crate::CheckpointReceipt,
    ) -> Result<std::fs::File, crate::CheckpointFailure> {
        use crate::CheckpointFailure;
        if self.host_directory.is_none() {
            return Err(CheckpointFailure::Unsupported);
        }
        crate::file_store::restore_lease(&self.state_root, &expected.instance_id)
            .map_err(|_| CheckpointFailure::Unconfirmed)
    }

    /// Discards every checkpoint a destroyed Instance published, naming each one that stays.
    pub(super) fn discard_checkpoints_of(&self, instance: &soma::InstanceId) {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            match crate::backend::published_checkpoints(instance) {
                Ok(published) => {
                    for (checkpoint, digest) in published {
                        if let Err(failure) =
                            self.discard_checkpoint(instance, &checkpoint, &digest)
                        {
                            eprintln!(
                                "soma: checkpoint {} of destroyed {} was not discarded: {failure:?}",
                                checkpoint.as_str(),
                                instance.as_str()
                            );
                        }
                    }
                }
                Err(failure) => eprintln!(
                    "soma: checkpoints of destroyed {} were not listed: {failure:?}",
                    instance.as_str()
                ),
            }
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = instance;
        }
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

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn receipt_digest(
    expected: &crate::CheckpointReceipt,
) -> Result<soma::GenerationId, crate::CheckpointFailure> {
    let bytes =
        serde_json::to_vec(expected).map_err(|_| crate::CheckpointFailure::CaptureRefused)?;
    let digest = soma_kvm::snapshot::Digest::of(&bytes);
    soma::GenerationId::new(format!("sha256:{digest}"))
        .map_err(|_| crate::CheckpointFailure::CaptureRefused)
}
