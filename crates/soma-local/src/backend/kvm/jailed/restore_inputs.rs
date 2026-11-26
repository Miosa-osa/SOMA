//! The inputs a checkpoint restore hands its worker beyond the checkpoint's own descriptors: the
//! private head cloned for it and the broker's attestations of what need not be read again.

use std::fs::File;

use soma::BackendFailureKind;

use super::build::Recovery;
use crate::backend::kvm::{boot::private_head_from, checkpoint::RestoreArtifacts};

impl Recovery {
    /// A private head for this Instance cloned from the checkpoint's saved overlay.
    pub(super) fn clone_head(
        checkpoint: &RestoreArtifacts,
        instance: &soma::InstanceId,
    ) -> Result<Option<File>, BackendFailureKind> {
        checkpoint
            .overlay
            .as_ref()
            .map(|template| {
                let template = template
                    .try_clone()
                    .map_err(|_| BackendFailureKind::Unavailable)?;
                private_head_from(template, instance)
            })
            .transpose()
    }

    /// The broker's attestations of the checkpoint memory, of the Generation root as its content
    /// was verified, and of the head this broker step has just cloned from the saved overlay.
    ///
    /// The head is vouched for here, after the clone and before any descriptor reaches the
    /// worker, and only under a private head root and while the saved overlay still has its
    /// recorded stamp. A launch from a Generation template builds no recovery, so it has none.
    pub(super) fn attestations(
        checkpoint: &RestoreArtifacts,
        head: Option<&File>,
        root: Option<soma_kvm::snapshot::ArtifactAttestation>,
        private_head_root: bool,
    ) -> soma_kvm::snapshot::RestoreAttestations {
        let head = checkpoint
            .overlay
            .as_ref()
            .zip(head)
            .filter(|_| private_head_root)
            .and_then(|(saved, head)| {
                crate::backend::kvm::checkpoint::vouch_head(
                    saved,
                    checkpoint.overlay_attestation,
                    head,
                )
            });
        let memory = checkpoint.memory_attestation;
        soma_kvm::snapshot::RestoreAttestations { memory, root, head }
    }
}

#[cfg(test)]
mod tests;
