use std::{fs::File, os::fd::OwnedFd};

use soma_vmm::{InstanceId, Machine, control::Reply, sandbox::WorkspaceChange};

use crate::channel::Channel;

pub(super) fn reply(
    machine: &mut Machine,
    control: &Channel,
    instance: InstanceId,
    attach: bool,
    mut descriptors: Vec<OwnedFd>,
) {
    let change = match (attach, descriptors.pop(), descriptors.is_empty()) {
        (false, None, true) => WorkspaceChange::Detach,
        (true, Some(file), true) => WorkspaceChange::Attach(File::from(file)),
        _ => {
            control.send(&Reply::Rejected("invalid workspace descriptor count").encode());
            return;
        }
    };
    match machine.change_workspace(instance, change) {
        Ok(outcome) => control.send(&Reply::Workspace(instance, outcome).encode()),
        Err(failure) => control.send(&Reply::Failure(&failure).encode()),
    }
}
