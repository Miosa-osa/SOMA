//! The worker end of one filesystem exchange. No host path is opened here.

use std::os::{fd::OwnedFd, unix::net::UnixStream};

use soma_vmm::{
    InstanceId, Machine,
    control::{Reply, read_file_request, write_file_answer},
};

use crate::channel::Channel;

pub(super) fn reply(
    machine: &mut Machine,
    control: &Channel,
    instance: InstanceId,
    descriptors: Vec<OwnedFd>,
) {
    let Ok([descriptor]) = <[_; 1]>::try_from(descriptors) else {
        control.send(&Reply::Rejected("file needs one private socket").encode());
        return;
    };
    let mut socket = UnixStream::from(descriptor);
    let Ok(operation) = read_file_request(&mut socket) else {
        control.send(&Reply::Rejected("invalid file request").encode());
        return;
    };
    match machine.file(instance, operation) {
        Ok(answer) if write_file_answer(&mut socket, &answer).is_ok() => {
            control.send(&Reply::FileComplete(instance).encode());
        }
        Ok(_) => control.send(&Reply::Rejected("file answer delivery failed").encode()),
        Err(failure) => control.send(&Reply::Failure(&failure).encode()),
    }
}
