use std::{fs::File, os::fd::OwnedFd, time::Duration};

use soma_vmm::{
    Machine,
    control::Reply,
    sandbox::{CheckpointJob, CheckpointJobError, CheckpointSource},
};

use crate::channel::Channel;

pub(super) fn reply(
    machine: &mut Machine,
    control: &Channel,
    source: CheckpointSource,
    descriptors: Vec<OwnedFd>,
) {
    let answer = match job(source, descriptors) {
        None => Reply::Rejected("invalid checkpoint descriptor count"),
        Some(job) => match machine.checkpoint(job, Duration::from_mins(15)) {
            Ok((instance, digest)) => Reply::Checkpointed(instance, *digest.as_bytes()),
            Err(CheckpointJobError::CaptureRefused) => {
                Reply::Rejected("checkpoint capture refused")
            }
            Err(CheckpointJobError::SourceUnconfirmed) => Reply::CheckpointUnconfirmed,
        },
    };
    control.send(&answer.encode());
}

fn job(source: CheckpointSource, descriptors: Vec<OwnedFd>) -> Option<CheckpointJob> {
    if descriptors.len() != 2 + usize::from(source.has_overlay) + usize::from(source.has_volume) {
        return None;
    }
    let mut descriptors = descriptors.into_iter().map(File::from);
    Some(CheckpointJob {
        source,
        state: descriptors.next()?,
        memory: descriptors.next()?,
        overlay: source.has_overlay.then(|| descriptors.next()).flatten(),
        volume: descriptors.next(),
    })
}
