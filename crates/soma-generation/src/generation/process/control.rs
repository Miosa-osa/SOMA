//! Process-group containment and descriptor naming for one external build tool.
//!
//! Every tool is started as the leader of a fresh process group, so one signal reaches the tool
//! and every descendant it forked rather than only the direct child.
//! Signalling a group has no safe standard-library equivalent, so this module owns the crate's
//! only `unsafe` call; nothing else in the compiler may signal a process.
//! On platforms without process groups the shims are inert, which is correct because the pinned
//! Linux build tools cannot run there at all.
//!
//! It also owns the one system-specific fact a pinned tool needs: the path that names an open
//! descriptor, so a measured executable can be run as measured rather than by name.

#![allow(unsafe_code)]

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Reserved group identifiers that are never signalled.
const LOWEST_SIGNALLABLE_GROUP: u32 = 2;

/// One tool's process group.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Group(u32);

/// Which signal one termination step sends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Signal {
    /// The polite request a well-behaved tool honors.
    Terminate,
    /// The signal a tool cannot catch, block, or ignore.
    Force,
}

impl Group {
    /// The group identifier a freshly spawned isolated child leads.
    ///
    /// `isolate` makes the child a group leader, so the group identifier equals its process
    /// identifier.
    pub(super) const fn new(child_id: u32) -> Self {
        Self(child_id)
    }

    /// Sends one signal to every member of the group.
    ///
    /// The caller must not signal after it has reaped the leader and every other member has
    /// exited, because the identifier is only reserved while the group still has a member.
    #[cfg(unix)]
    pub(super) fn signal(self, signal: Signal) {
        if self.0 < LOWEST_SIGNALLABLE_GROUP {
            return;
        }
        let Ok(group) = i32::try_from(self.0) else {
            return;
        };
        let number = match signal {
            Signal::Terminate => libc::SIGTERM,
            Signal::Force => libc::SIGKILL,
        };
        // SAFETY: `kill` has no memory preconditions. The negative identifier selects one
        // process group, which is above the reserved identifiers and was created by `isolate`
        // for this invocation, so the call can only reach the tool and its descendants.
        unsafe { libc::kill(-group, number) };
    }

    #[cfg(not(unix))]
    pub(super) fn signal(self, _signal: Signal) {
        let _ = (self.0, LOWEST_SIGNALLABLE_GROUP);
    }
}

/// Starts the tool as the leader of a fresh process group.
#[cfg(unix)]
pub(super) fn isolate(command: &mut Command) {
    use std::os::unix::process::CommandExt as _;
    command.process_group(0);
}

#[cfg(not(unix))]
pub(super) fn isolate(_command: &mut Command) {}

/// Lets the child keep the verified tool descriptor across `execve`.
///
/// The parent keeps its copy close-on-exec, so the descriptor never reaches an unrelated
/// process; only the child that is about to execute those exact bytes inherits it.
/// A `#!` tool needs this, because its interpreter opens the descriptor path after the first
/// `execve` has already applied close-on-exec.
#[cfg(unix)]
pub(super) fn inherit_tool(command: &mut Command, tool: std::os::unix::io::RawFd) {
    use std::os::unix::process::CommandExt as _;

    // SAFETY: the closure runs in the forked child between `fork` and `execve`. It calls only
    // `fcntl`, which is async-signal-safe, allocates nothing, and touches no shared state.
    unsafe {
        command.pre_exec(move || {
            if libc::fcntl(tool, libc::F_SETFD, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

/// Inherits nothing on a system without descriptor paths.
#[cfg(not(unix))]
pub(super) fn inherit_tool(_command: &mut Command, _tool: i32) {}

/// Returns the path that names one of this process's open descriptors.
///
/// Linux publishes every descriptor under `/proc/self/fd`, and opening that path re-opens the
/// object the descriptor holds rather than resolving the original name again.
#[cfg(target_os = "linux")]
pub(super) fn descriptor_path(file: &File) -> PathBuf {
    use std::os::unix::io::AsRawFd as _;

    PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

/// Returns the same descriptor path on the systems that publish `/dev/fd`.
#[cfg(all(unix, not(target_os = "linux")))]
pub(super) fn descriptor_path(file: &File) -> PathBuf {
    use std::os::unix::io::AsRawFd as _;

    PathBuf::from(format!("/dev/fd/{}", file.as_raw_fd()))
}

/// Returns a name no descriptor can have, because this system publishes no descriptor paths.
///
/// `require_same_object` refuses every path here, so a pinned tool fails as unsupported rather
/// than falling back to executing a name.
#[cfg(not(unix))]
pub(super) fn descriptor_path(_file: &File) -> PathBuf {
    PathBuf::from("soma-descriptor-paths-are-unavailable")
}

/// Proves that `path` reaches exactly the object `file` already holds.
///
/// # Errors
///
/// Returns `Err(())` when the path cannot be inspected or names a different object, which is
/// what a system whose descriptor paths do not work looks like.
#[cfg(unix)]
pub(super) fn require_same_object(file: &File, path: &Path) -> Result<(), ()> {
    use std::os::unix::fs::MetadataExt as _;

    let held = file.metadata().map_err(|_| ())?;
    let named = std::fs::metadata(path).map_err(|_| ())?;
    if held.dev() == named.dev() && held.ino() == named.ino() {
        Ok(())
    } else {
        Err(())
    }
}

/// Refuses every path on a system without descriptor paths.
#[cfg(not(unix))]
pub(super) fn require_same_object(_file: &File, _path: &Path) -> Result<(), ()> {
    Err(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_group_identifiers_are_never_signalled() {
        Group::new(0).signal(Signal::Terminate);
        Group::new(1).signal(Signal::Force);
        assert_eq!(Group::new(7), Group::new(7));
        assert_ne!(Group::new(7), Group::new(8));
    }

    #[test]
    fn a_descriptor_path_reaches_the_object_the_descriptor_holds() {
        let file = File::open("/bin/sh").expect("a host shell");
        let path = descriptor_path(&file);

        assert!(require_same_object(&file, &path).is_ok());
        assert!(require_same_object(&file, Path::new("/dev/null")).is_err());
        assert!(require_same_object(&file, Path::new("/soma-no-such-path")).is_err());
    }
}
