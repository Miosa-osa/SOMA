use std::{fs::File, os::fd::OwnedFd};

use soma_kvm::snapshot::Digest;
use soma_vmm::{Launch, Machine, control::Reply};

use crate::channel::Channel;

pub(super) fn reply(
    machine: &mut Machine,
    control: &Channel,
    launch: Launch,
    digest: [u8; 32],
    descriptors: Vec<OwnedFd>,
) {
    if descriptors.len() != usize::from(launch.generation().devices().writable_disk()) {
        control.send(&Reply::Rejected("invalid recovery descriptor count").encode());
        return;
    }
    let saved = descriptors.into_iter().next().map(File::from);
    match machine.recover(launch, Digest::from_bytes(digest), saved) {
        Ok(ready) => control.send(&Reply::Ready(&ready).encode()),
        Err(failure) => control.send(&Reply::Failure(&failure).encode()),
    }
}
