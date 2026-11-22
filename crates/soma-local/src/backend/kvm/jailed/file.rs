//! Filesystem traffic crosses a one-use socket, so the jailed worker never takes a host path.

use std::{
    net::Shutdown,
    os::{fd::AsFd, unix::net::UnixStream},
    time::Instant,
};

use soma::{BackendFailureKind, FileAnswer, FileOperation};
use soma_vmm::control::{
    FILE_CONTROL_CEILING, Outcome, Request, read_file_answer, write_file_request,
};

use super::Jailed;

impl Jailed {
    pub(in crate::backend::kvm) fn file(
        &mut self,
        operation: &FileOperation,
    ) -> Result<FileAnswer, BackendFailureKind> {
        if self.poisoned {
            return Err(BackendFailureKind::Unavailable);
        }
        let (mut local, peer) = UnixStream::pair().map_err(|_| BackendFailureKind::Unavailable)?;
        local
            .set_read_timeout(Some(FILE_CONTROL_CEILING))
            .and_then(|()| local.set_write_timeout(Some(FILE_CONTROL_CEILING)))
            .map_err(|_| BackendFailureKind::Unavailable)?;
        let start = Instant::now();
        self.control
            .tell_with_files(&Request::File(self.instance), &[peer.as_fd()])
            .map_err(|kind| self.poison(kind))?;
        drop(peer);
        let result = write_file_request(&mut local, operation)
            .and_then(|()| local.shutdown(Shutdown::Write))
            .and_then(|()| read_file_answer(&mut local));
        // An older worker closes the socket and acknowledges an unknown request. This
        // is a capability refusal, so it must not poison a machine that is still usable.
        let remaining = FILE_CONTROL_CEILING.saturating_sub(start.elapsed());
        match self
            .control
            .receive(remaining)
            .and_then(|reply| Outcome::decode(&reply).map_err(|_| BackendFailureKind::Unavailable))
        {
            Ok(Outcome::FileComplete(instance)) if instance == self.instance => {
                result.map_err(|_| self.poison(BackendFailureKind::GuestFailure))
            }
            Ok(Outcome::Rejected(reason)) if reason == "unknown request" => {
                Err(BackendFailureKind::Unsupported)
            }
            _ => Err(self.poison(BackendFailureKind::GuestFailure)),
        }
    }
}
