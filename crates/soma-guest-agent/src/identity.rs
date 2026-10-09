//! Identity repair: hostname, machine identity, session state, and wall clock.
//!
//! Every value derives from the fresh `InstanceId` or the launch-page time sample so a
//! restored clone never presents the captured identity.

#![allow(unsafe_code)]

use std::fmt::Write;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;

use crate::boot::Declared;
use crate::mounts;
use crate::tuning;

const HOSTNAME_SYSCTL: &str = "/proc/sys/kernel/hostname";
const HOSTNAME_FILE: &str = "/etc/hostname";
const MACHINE_ID_FILE: &str = "/etc/machine-id";
const MACHINE_ID_STAGING: &str = "/etc/.machine-id.soma";
const HOSTNAME_PREFIX: &str = "soma-";
// The session filesystems carry only option strings: `nosuid` and `nodev` are mount flags, not
// parameters, and the new mount API rejects them inside the option string.

/// Process state, never a workload, so its 16 MiB has never needed to move.
const RUN_DIRECTORY: &str = "/run";
/// The options `/run` is mounted with.
const RUN_OPTIONS: &str = "mode=0755,size=16m";
/// The shared scratch filesystem, where a workload writes its workspace.
const SCRATCH_DIRECTORY: &str = "/tmp";
/// The tmpfs `/tmp` is mounted as, on a machine that has no writable root to put it on.
const SCRATCH_TMPFS_OPTIONS: &str = "mode=1777,size=64m";
/// The permissions the scratch directory is given.
const SCRATCH_MODE: u32 = 0o1777;
const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// Redacted identity-repair failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityError {
    /// The kernel hostname could not be replaced.
    Hostname(i32),
    /// The machine identity file could not be replaced atomically.
    MachineId(i32),
    /// A captured session directory could not be replaced by a fresh tmpfs.
    SessionState(i32),
    /// The wall clock could not be set from the launch sample.
    Clock(i32),
}

/// Derives the guest hostname from the Instance identity.
#[must_use]
pub fn hostname(instance: &[u8; 16]) -> String {
    let mut name = String::from(HOSTNAME_PREFIX);
    for byte in &instance[..6] {
        let _ = write!(name, "{byte:02x}");
    }
    name
}

/// Derives the 32-character machine identity from the Instance identity.
#[must_use]
pub fn machine_id(instance: &[u8; 16]) -> String {
    let mut id = String::with_capacity(33);
    for byte in instance {
        let _ = write!(id, "{byte:02x}");
    }
    id.push('\n');
    id
}

/// Splits a Unix-nanosecond sample into a `timespec` pair.
#[must_use]
pub fn timespec(nanos: u64) -> (i64, i64) {
    let seconds = i64::try_from(nanos / NANOS_PER_SECOND).unwrap_or(i64::MAX);
    let remainder = i64::try_from(nanos % NANOS_PER_SECOND).unwrap_or(0);
    (seconds, remainder)
}

/// Replaces hostname, machine identity, session directories, and the wall clock.
///
/// # Errors
///
/// Returns the first failed step with its errno.
pub fn repair(
    instance: &[u8; 16],
    time_sample_nanos: u64,
    declared: Declared,
) -> Result<(), IdentityError> {
    let name = hostname(instance);
    // The kernel hostname lives in procfs and is replaced whatever the root is made of; it is
    // what every process actually reads through `uname` and `gethostname`.
    fs::write(HOSTNAME_SYSCTL, name.as_bytes())
        .map_err(|error| IdentityError::Hostname(errno(&error)))?;
    // The two files under `/etc` are copies of identity the kernel already holds, and a machine
    // with no writable root has nowhere to put them. They are written when there is a private
    // overlay to write them to and skipped when there is not, rather than failing a boot over a
    // file the Generation deliberately made unwritable.
    if declared.overlay {
        fs::write(HOSTNAME_FILE, format!("{name}\n"))
            .map_err(|error| IdentityError::Hostname(errno(&error)))?;
        fs::write(MACHINE_ID_STAGING, machine_id(instance))
            .and_then(|()| {
                fs::set_permissions(MACHINE_ID_STAGING, fs::Permissions::from_mode(0o444))
            })
            .and_then(|()| fs::rename(MACHINE_ID_STAGING, MACHINE_ID_FILE))
            .map_err(|error| IdentityError::MachineId(errno(&error)))?;
    }
    // `/run` is a tmpfs either way: process state, never a workload, and it is what gives a
    // read-only sandbox a writable `/run` at all.
    reset_session_directory(RUN_DIRECTORY, RUN_OPTIONS)?;
    // `/tmp` is where a workload writes its workspace, so where it lives is a sizing decision
    // rather than a session one. See [`scratch_on_disk`].
    if scratch_on_disk(declared.overlay, tuning::total_memory_bytes()) {
        reset_scratch_directory(SCRATCH_DIRECTORY)?;
    } else {
        reset_session_directory(SCRATCH_DIRECTORY, SCRATCH_TMPFS_OPTIONS)?;
    }
    set_clock(time_sample_nanos)
}

/// Whether this machine keeps `/tmp` on its writable root rather than in RAM.
///
/// A tmpfs is charged to guest RAM. On the large shape, which is where a build writes a
/// workspace, that is RAM the build cannot have: measured 2026-10-08, a large DAX run with `/tmp`
/// in RAM was killed by the OOM killer about one run in three, and Firecracker keeps `/tmp` on
/// disk for the same reason. The private overlay is where those bytes were always going to live,
/// so the large shape puts them there.
///
/// Both halves are needed. The size decides the policy, because the version 1 machine keeps the
/// tmpfs it has always had; the overlay decides whether the policy is possible at all, because a
/// machine with no writable root has nowhere else to put `/tmp` and a read-only root would leave
/// it unusable.
#[must_use]
pub const fn scratch_on_disk(overlay: bool, total_bytes: Option<u64>) -> bool {
    match total_bytes {
        Some(total) => overlay && tuning::sizing_applies(total),
        None => false,
    }
}

/// Empties the scratch directory and gives it back at the mode a tmpfs would have had.
///
/// A tmpfs mounted over `/tmp` hid whatever the image and the Generation's warm commands left
/// there, so no Instance ever saw it. A directory on the writable root has to be emptied for the
/// same reason, or a warmed file becomes captured state that every Instance inherits.
fn reset_scratch_directory(directory: &str) -> Result<(), IdentityError> {
    match fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(directory)
            .map_err(|error| IdentityError::SessionState(errno(&error)))?,
        Ok(_) => fs::remove_file(directory)
            .map_err(|error| IdentityError::SessionState(errno(&error)))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(IdentityError::SessionState(errno(&error))),
    }
    fs::create_dir_all(directory).map_err(|error| IdentityError::SessionState(errno(&error)))?;
    fs::set_permissions(directory, fs::Permissions::from_mode(SCRATCH_MODE))
        .map_err(|error| IdentityError::SessionState(errno(&error)))
}

fn reset_session_directory(directory: &str, options: &str) -> Result<(), IdentityError> {
    if fs::symlink_metadata(directory).is_ok_and(|metadata| !metadata.is_dir()) {
        fs::remove_file(directory).map_err(|error| IdentityError::SessionState(errno(&error)))?;
    }
    fs::create_dir_all(directory).map_err(|error| IdentityError::SessionState(errno(&error)))?;
    mounts::mount(
        "tmpfs",
        directory,
        "tmpfs",
        libc::MS_NOSUID | libc::MS_NODEV,
        options,
    )
    .map_err(|error| IdentityError::SessionState(error.0))
}

fn set_clock(time_sample_nanos: u64) -> Result<(), IdentityError> {
    let (seconds, nanos) = timespec(time_sample_nanos);
    let time = libc::timespec {
        tv_sec: seconds,
        tv_nsec: nanos,
    };
    // SAFETY: `clock_settime` reads one valid `timespec` local for the real-time clock.
    let result = unsafe { libc::clock_settime(libc::CLOCK_REALTIME, &raw const time) };
    if result == 0 {
        Ok(())
    } else {
        Err(IdentityError::Clock(errno(&io::Error::last_os_error())))
    }
}

fn errno(error: &io::Error) -> i32 {
    error.raw_os_error().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSTANCE: [u8; 16] = [
        0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b,
        0x0c,
    ];

    #[test]
    fn hostname_is_a_short_instance_derived_label() {
        let name = hostname(&INSTANCE);
        assert_eq!(name, "soma-deadbeef0102");
        assert!(name.len() <= 63);
        assert!(
            name.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        );
    }

    #[test]
    fn machine_id_is_thirty_two_lowercase_hex_digits() {
        let id = machine_id(&INSTANCE);
        assert_eq!(id, "deadbeef0102030405060708090a0b0c\n");
        assert_eq!(id.trim_end().len(), 32);
        assert_ne!(machine_id(&[1; 16]), machine_id(&[2; 16]));
    }

    #[test]
    fn time_samples_split_into_seconds_and_nanoseconds() {
        assert_eq!(
            timespec(1_700_000_000_123_456_789),
            (1_700_000_000, 123_456_789)
        );
        assert_eq!(timespec(0), (0, 0));
        assert!(timespec(u64::MAX).1 < 1_000_000_000);
    }

    #[test]
    fn the_scratch_filesystem_moves_to_disk_only_for_a_large_machine_that_has_one() {
        // The large shape is where a workspace is written, and a tmpfs would charge it to guest
        // RAM. Above the version 1 ceiling, and only with a writable root to put it on.
        assert!(scratch_on_disk(
            true,
            Some(16 * 1024 * 1024 * 1024 - 4096 * 1024)
        ));
        assert!(scratch_on_disk(true, Some(3 * 1024 * 1024 * 1024 + 1)));
        // A version 1 machine keeps the 64 MiB tmpfs it has always had.
        assert!(!scratch_on_disk(true, Some(256 * 1024 * 1024)));
        assert!(!scratch_on_disk(true, Some(3 * 1024 * 1024 * 1024)));
        // A read-only root has nowhere else to put `/tmp`, so the tmpfs stays whatever the size.
        assert!(!scratch_on_disk(false, Some(16 * 1024 * 1024 * 1024)));
        // A kernel that does not report its memory is not a machine whose shape we know.
        assert!(!scratch_on_disk(true, None));
    }

    #[test]
    fn setting_the_clock_without_privilege_fails_closed() {
        // As root the call would succeed and step the host's real-time clock, so the test only
        // runs where it is what it claims to be: unprivileged.
        // SAFETY: `geteuid` has no preconditions.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        assert!(matches!(
            set_clock(1_700_000_000_000_000_000),
            Err(IdentityError::Clock(libc::EPERM))
        ));
    }
}
