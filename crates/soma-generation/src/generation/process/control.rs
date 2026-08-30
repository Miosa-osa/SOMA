//! Descriptor naming and inheritance for one pinned external build tool.
//!
//! Process-group containment belongs to [`soma_supervise`]; this module owns the one
//! system-specific fact a pinned tool needs: the path that names an open descriptor, so a
//! measured executable can be run as measured rather than by name.
//! Keeping that descriptor across `execve` has no safe standard-library equivalent, so this
//! module owns the compiler's only `unsafe` process call.

#![allow(unsafe_code)]

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    fn a_descriptor_path_reaches_the_object_the_descriptor_holds() {
        let file = File::open("/bin/sh").expect("a host shell");
        let path = descriptor_path(&file);

        assert!(require_same_object(&file, &path).is_ok());
        assert!(require_same_object(&file, Path::new("/dev/null")).is_err());
        assert!(require_same_object(&file, Path::new("/soma-no-such-path")).is_err());
    }
}
