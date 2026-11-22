use std::os::fd::AsFd as _;

use soma::BackendFailureKind;
use soma_kvm::snapshot::Digest;
use soma_vmm::{
    control::{Outcome, Request},
    sandbox::CheckpointJob,
};

use crate::CheckpointFailure;

use super::Jailed;

impl Jailed {
    pub(in crate::backend::kvm) fn checkpoint(
        &mut self,
        job: &CheckpointJob,
    ) -> Result<Digest, CheckpointFailure> {
        if !self.is_usable() || self.instance.as_bytes() != &job.source.instance_id {
            return Err(CheckpointFailure::Unconfirmed);
        }
        let mut descriptors = vec![job.state.as_fd(), job.memory.as_fd()];
        if let Some(overlay) = job.overlay.as_ref() {
            descriptors.push(overlay.as_fd());
        }
        if let Some(volume) = job.volume.as_ref() {
            descriptors.push(volume.as_fd());
        }
        let reply = self.control.ask_with_files(
            &Request::Checkpoint(job.source),
            &descriptors,
            super::super::checkpoint::CHECKPOINT_CEILING + std::time::Duration::from_secs(15),
        );
        match reply {
            Ok(Outcome::Checkpointed(instance, digest)) if instance == self.instance => {
                Ok(Digest::from_bytes(digest))
            }
            Ok(Outcome::Rejected(reason)) if reason == "unknown request" => {
                Err(CheckpointFailure::Unsupported)
            }
            Ok(Outcome::Rejected(_)) => Err(CheckpointFailure::CaptureRefused),
            _ => {
                self.poison(BackendFailureKind::GuestFailure);
                Err(CheckpointFailure::Unconfirmed)
            }
        }
    }
}
