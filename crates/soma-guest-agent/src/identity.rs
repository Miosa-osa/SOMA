//! Identity repair: hostname, machine identity, session state, and wall clock.
//!
//! Every value derives from the fresh `InstanceId` or the launch-page time sample so a
//! restored clone never presents the captured identity.

#![allow(unsafe_code)]

use std::fmt::Write;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;

use crate::mounts;

const HOSTNAME_SYSCTL: &str = "/proc/sys/kernel/hostname";
const HOSTNAME_FILE: &str = "/etc/hostname";
const MACHINE_ID_FILE: &str = "/etc/machine-id";
const MACHINE_ID_STAGING: &str = "/etc/.machine-id.soma";
const HOSTNAME_PREFIX: &str = "soma-";
const SESSION_TMPFS: [(&str, &str); 2] = [
    ("/run", "mode=0755,size=16m,nosuid,nodev"),
    ("/tmp", "mode=1777,size=64m,nosuid,nodev"),
];
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
pub fn repair(instance: &[u8; 16], time_sample_nanos: u64) -> Result<(), IdentityError> {
    let name = hostname(instance);
    fs::write(HOSTNAME_SYSCTL, name.as_bytes())
        .map_err(|error| IdentityError::Hostname(errno(&error)))?;
    fs::write(HOSTNAME_FILE, format!("{name}\n"))
        .map_err(|error| IdentityError::Hostname(errno(&error)))?;
    fs::write(MACHINE_ID_STAGING, machine_id(instance))
        .and_then(|()| fs::set_permissions(MACHINE_ID_STAGING, fs::Permissions::from_mode(0o444)))
        .and_then(|()| fs::rename(MACHINE_ID_STAGING, MACHINE_ID_FILE))
        .map_err(|error| IdentityError::MachineId(errno(&error)))?;
    for (directory, options) in SESSION_TMPFS {
        reset_session_directory(directory, options)?;
    }
    set_clock(time_sample_nanos)
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
    fn setting_the_clock_without_privilege_fails_closed() {
        assert!(matches!(
            set_clock(1_700_000_000_000_000_000),
            Err(IdentityError::Clock(libc::EPERM))
        ));
    }
}
