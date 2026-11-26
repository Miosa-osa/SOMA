//! A guest that shuts down or restarts from inside (SOMA#258).
//!
//! The guest ends its machine on its own, so nothing asks this host about it: a watcher thread
//! learns of it, either from this process's own sandbox thread or from the jailed worker's exit
//! status, and tells the serve loop over the host's own socket, as the idle watcher does. The
//! loop then answers on the one thread that owns the machine. A restart starts the same Instance
//! again in place, with the same volume and request, so the desktop comes back by itself. A
//! shutdown releases the machine and leaves a marker beside the socket, so reading the Instance
//! says the guest shut it down rather than that its machine was lost.

use std::{
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use soma_kvm::x86_64::GuestEnd;
use soma_vmm::sandbox::guest_end;

use soma::BackendFailureKind;

use crate::backend::kvm::{KvmBackend, lifecycle::LaunchExtras, prepared::PreparedGeneration};

use super::super::{
    Launched, channel,
    wire::{Call, LaunchWire},
};

/// How often the watcher looks for the guest's own end.
const POLL: Duration = Duration::from_millis(250);

/// The marker a guest shutdown leaves beside the host's socket.
pub(in crate::backend::kvm) fn shutdown_marker(socket: &Path) -> PathBuf {
    socket.with_extension("guest-shutdown")
}

/// Starts the same Instance again after the guest restarted it: a fresh boot of the same
/// Generation with the same volume and shape, never the checkpoint or suspension it may first
/// have come from, which would undo everything since.
pub(super) fn relaunch(
    backend: &mut KvmBackend,
    request: &LaunchWire,
    prepared: &PreparedGeneration,
) -> Result<Launched, BackendFailureKind> {
    backend
        .launch_resident(
            &request.operation_id,
            &request.instance_id,
            prepared,
            &request.shape,
            LaunchExtras {
                volume: request.volume.clone(),
                windowed: request.windowed,
                checkpoint: None,
                resume: None,
            },
        )
        .map_err(|failure| failure.kind())
}

/// Starts the watcher for the machine just launched; `worker` is its jailed worker, if any.
pub(super) fn watch(socket: PathBuf, worker: Option<i32>) {
    let _ignored = thread::Builder::new()
        .name("soma-machine-guest-end".to_owned())
        .spawn(move || {
            loop {
                thread::sleep(POLL);
                let end = match worker {
                    None => guest_end::take_observed(),
                    Some(pid) => match worker_status(pid) {
                        Worker::Running => None,
                        Worker::Ended(end) => end,
                        Worker::Gone => return,
                    },
                };
                let Some(end) = end else {
                    if worker.is_some() {
                        // A worker that exited for any other reason was ended by this host.
                        return;
                    }
                    continue;
                };
                if let Ok(mut stream) = UnixStream::connect(&socket) {
                    let reboot = end == GuestEnd::Reboot;
                    let _ignored = channel::write_line(&mut stream, &Call::GuestEnded { reboot });
                }
                return;
            }
        });
}

enum Worker {
    Running,
    /// The worker exited; with the guest's own end when its status names one.
    Ended(Option<GuestEnd>),
    /// The worker was already reaped by the release that ended it.
    Gone,
}

/// Reads the worker's exit without reaping it, so the release that follows still owns the wait.
fn worker_status(pid: i32) -> Worker {
    use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};

    let Some(pid) = Pid::from_raw(pid) else {
        return Worker::Gone;
    };
    let options = WaitIdOptions::EXITED | WaitIdOptions::NOWAIT | WaitIdOptions::NOHANG;
    match waitid(WaitId::Pid(pid), options) {
        Ok(None) => Worker::Running,
        Ok(Some(status)) => Worker::Ended(
            status
                .exit_status()
                .filter(|_| status.exited())
                .and_then(guest_end::from_status),
        ),
        Err(_) => Worker::Gone,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::shutdown_marker;

    /// The host writes the marker the runtime reads.
    #[test]
    fn the_host_and_the_runtime_name_one_marker() {
        let directory = Path::new("/run/soma");
        let instance = soma::InstanceId::new("0123456789abcdef0123456789abcdef").expect("id");
        assert_eq!(
            shutdown_marker(&super::super::super::channel::socket_path(
                directory, &instance
            )),
            crate::backend::guest_shutdown_marker(directory, &instance)
        );
    }

    #[test]
    fn the_marker_sits_beside_the_socket() {
        assert_eq!(
            shutdown_marker(Path::new("/run/soma/abc.sock")),
            Path::new("/run/soma/abc.guest-shutdown")
        );
    }
}
