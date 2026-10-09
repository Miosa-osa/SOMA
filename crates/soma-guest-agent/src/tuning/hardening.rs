//! G1 and G2: the boot-log clear before the capture point and the lockdown block.
//!
//! A Generation is captured after boot, so anything the golden boot left in the kernel ring buffer
//! is copied into every restored Instance. Clearing it before the capture keeps the guest's own
//! log empty for the workload that later runs there.
//!
//! The lockdown block narrows the information surface a workload can read about the machine it is
//! sharing: the kernel log and pointer addresses through the `dmesg_restrict`, `kptr_restrict`, and
//! `perf_event_paranoid` sysctls, then the two procfs and device files that carry the same facts,
//! then `/proc/version`, which names the exact kernel build. None of this is a boundary against
//! root inside the guest; it is what a non-root workload is allowed to learn.

#![allow(unsafe_code)]

use std::{fs, os::unix::fs::PermissionsExt as _, path::Path};

use crate::mounts;

use super::write_sysctl;

/// The private tmpfs the cover step mounts, and the empty file on it that covers the version file.
const COVER_DIR: &str = "/dev/.soma-lockdown";
const COVER_FILE: &str = "/dev/.soma-lockdown/version";
/// The procfs file whose content the cover hides.
const PROC_VERSION: &str = "/proc/version";
/// Files whose read permission is removed outright.
const HIDDEN: [&str; 2] = ["/dev/kmsg", "/proc/config.gz"];
/// The `SYSLOG_ACTION_CLEAR` request of the kernel's `syslog` syscall.
const SYSLOG_ACTION_CLEAR: libc::c_int = 5;

/// Applies the lockdown block and covers the version file.
///
/// # Errors
///
/// Returns a short reason for the first sysctl or the cover mount that could not be applied. The
/// per-file mode change is best effort and never fails the step: root keeps `CAP_DAC_OVERRIDE`
/// regardless, so it narrows what a non-root workload reads and nothing else.
pub(super) fn lockdown() -> Result<(), String> {
    for (name, value) in [
        ("kernel/dmesg_restrict", 1),
        ("kernel/kptr_restrict", 1),
        ("kernel/perf_event_paranoid", 2),
    ] {
        write_sysctl(name, value)?;
    }
    for path in HIDDEN {
        if Path::new(path).exists() {
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o000));
        }
    }
    cover_proc_version()
}

/// Drops everything the golden boot wrote into the kernel ring buffer.
///
/// Called immediately before the capture point, so no restored Instance carries the boot log.
pub fn clear_boot_log() {
    // SAFETY: the kernel `syslog` syscall with `SYSLOG_ACTION_CLEAR` takes no buffer and only
    // empties the ring buffer; a null pointer and a zero length are its no-argument form.
    unsafe {
        libc::syscall(
            libc::SYS_syslog,
            SYSLOG_ACTION_CLEAR,
            std::ptr::null::<u8>(),
            0,
        );
    }
}

/// Mounts an empty in-memory file over `/proc/version`.
///
/// A tmpfs cannot cover a single file, so the cover is a tmpfs holding one empty file, bind
/// mounted over the target. `/dev` is devtmpfs and writable, so the mount point needs nothing
/// from the possibly read-only composed root.
fn cover_proc_version() -> Result<(), String> {
    fs::create_dir_all(COVER_DIR).map_err(|error| short("cover-dir", &error))?;
    match mounts::mount(
        "tmpfs",
        COVER_DIR,
        "tmpfs",
        libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
        "size=8k",
    ) {
        Ok(()) | Err(mounts::Errno(libc::EBUSY)) => {}
        Err(errno) => return Err(format!("cover-mount:{}", errno.0)),
    }
    fs::write(COVER_FILE, b"").map_err(|error| short("cover-file", &error))?;
    mounts::bind_mount(COVER_FILE, PROC_VERSION).map_err(|errno| format!("cover-bind:{}", errno.0))
}

/// Ensures `/etc/hosts` names localhost.
///
/// A machine with no network device raises loopback and writes no hosts file, and an image may not
/// even carry `/etc/hosts`, because a container runtime usually creates it at run time. A workload
/// that resolves `localhost` then finds nothing. Measured 2026-10-08: the large shape, which
/// declares no network device, reported no `localhost` line until this step ran. The step is
/// idempotent and leaves a file that already names localhost alone.
///
/// # Errors
///
/// Returns a short reason if the file could not be written.
pub(super) fn ensure_localhost() -> Result<(), String> {
    const HOSTS: &str = "/etc/hosts";
    let existing = fs::read_to_string(HOSTS).unwrap_or_default();
    let named = existing
        .lines()
        .any(|line| line.split_whitespace().any(|word| word == "localhost"));
    if named {
        return Ok(());
    }
    let mut text = existing;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str("127.0.0.1 localhost\n");
    fs::write(HOSTS, text).map_err(|error| short("hosts", &error))
}

/// Renders a labelled errno, matching the shape the sysctl writer reports.
fn short(label: &str, error: &std::io::Error) -> String {
    format!("{label}:{}", error.raw_os_error().unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clear_request_is_the_kernel_clear_action() {
        // SYSLOG_ACTION_CLEAR is 5 in the kernel's `syslog` interface; a different value clears a
        // different buffer, so the constant is pinned here.
        assert_eq!(SYSLOG_ACTION_CLEAR, 5);
    }

    #[test]
    fn the_cover_lives_on_a_devtmpfs_directory() {
        assert!(COVER_DIR.starts_with("/dev/"));
        assert!(COVER_FILE.starts_with(COVER_DIR));
        assert_eq!(PROC_VERSION, "/proc/version");
    }
}
