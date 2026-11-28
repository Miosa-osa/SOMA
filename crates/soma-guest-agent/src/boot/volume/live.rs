//! Mount transitions for the one durable workspace slot declared by the Generation.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt as _;

use soma_guest::{WorkspaceAction, WorkspaceOutcome};

use super::{
    BootFailure, PERSISTENCE_PATHS, VOLUME_DEVICE, VOLUME_MOUNT, bind_path, compose, mount_device,
    mount_ext4, mounts,
};

enum State {
    Undeclared,
    Declared,
    Mounted,
    Offline(File),
    Inconsistent,
}

/// Guest ownership of the declared workspace throughout the machine's lifetime.
pub struct Workspace(State);

impl Workspace {
    /// Creates the mount state from the immutable Generation declaration.
    pub const fn new(declared: bool) -> Self {
        Self(if declared {
            State::Declared
        } else {
            State::Undeclared
        })
    }

    /// Mounts the initially assigned workspace before readiness.
    ///
    /// # Errors
    ///
    /// Returns the first failed device or mount check.
    pub fn boot(&mut self) -> Result<(), BootFailure> {
        if matches!(self.0, State::Declared) {
            // A launch that named no volume has none to change later either.
            self.0 = if compose()? {
                State::Mounted
            } else {
                State::Undeclared
            };
        }
        Ok(())
    }

    /// Changes the mounted state in response to the authenticated host.
    pub fn perform(&mut self, action: WorkspaceAction) -> WorkspaceOutcome {
        match (&self.0, action) {
            (State::Undeclared | State::Declared, _) => WorkspaceOutcome::Unsupported,
            (State::Inconsistent, _) => WorkspaceOutcome::Inconsistent,
            (State::Mounted, WorkspaceAction::Mount)
            | (State::Offline(_), WorkspaceAction::Unmount) => WorkspaceOutcome::Ready,
            (State::Mounted, WorkspaceAction::Unmount) => self.unmount(),
            (State::Offline(_), WorkspaceAction::Mount) => self.mount(),
        }
    }

    fn unmount(&mut self) -> WorkspaceOutcome {
        for (index, path) in PERSISTENCE_PATHS.into_iter().enumerate().rev() {
            if mounts::unmount(path).is_err() {
                return self.restore_binds(&PERSISTENCE_PATHS[index + 1..]);
            }
        }
        if mounts::unmount(VOLUME_MOUNT).is_err() {
            return self.restore_binds(&PERSISTENCE_PATHS);
        }
        match claim_device() {
            Ok(lease) => {
                self.0 = State::Offline(lease);
                WorkspaceOutcome::Ready
            }
            Err(_) if mount_ext4().is_ok() => self.restore_binds(&PERSISTENCE_PATHS),
            Err(_) => self.inconsistent(),
        }
    }

    fn mount(&mut self) -> WorkspaceOutcome {
        let previous = std::mem::replace(&mut self.0, State::Inconsistent);
        if let State::Offline(lease) = previous {
            drop(lease);
        }
        if mount_device().is_err() {
            return self.reclaim();
        }
        for (index, path) in PERSISTENCE_PATHS.into_iter().enumerate() {
            if bind_path(path).is_err() {
                let rollback = PERSISTENCE_PATHS[..index]
                    .iter()
                    .rev()
                    .all(|bound| mounts::unmount(bound).is_ok());
                return if rollback && mounts::unmount(VOLUME_MOUNT).is_ok() {
                    self.reclaim()
                } else {
                    self.inconsistent()
                };
            }
        }
        self.0 = State::Mounted;
        WorkspaceOutcome::Ready
    }

    fn reclaim(&mut self) -> WorkspaceOutcome {
        if let Ok(lease) = claim_device() {
            self.0 = State::Offline(lease);
            WorkspaceOutcome::Refused
        } else {
            self.inconsistent()
        }
    }

    fn restore_binds(&mut self, paths: &[&str]) -> WorkspaceOutcome {
        if paths.iter().all(|path| bind_path(path).is_ok()) {
            WorkspaceOutcome::Refused
        } else {
            self.inconsistent()
        }
    }

    fn inconsistent(&mut self) -> WorkspaceOutcome {
        self.0 = State::Inconsistent;
        WorkspaceOutcome::Inconsistent
    }
}

fn claim_device() -> io::Result<File> {
    // Linux's exclusive block open also detects filesystem holders in other mount namespaces.
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_EXCL | libc::O_NOFOLLOW)
        .open(VOLUME_DEVICE)
}
