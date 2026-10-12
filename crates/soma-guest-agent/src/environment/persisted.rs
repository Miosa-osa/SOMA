//! The environment a sandbox keeps between commands.
//!
//! `/etc/miosa/identity.env` is written by whoever configures the sandbox (the control plane
//! writes it through the filesystem API, the same file envd has always kept) and holds
//! `NAME='value'` lines. The agent applies it as the base environment of every direct command
//! and every terminal: above the agent's fixed base, below whatever the command itself names.
//! It is read at each command, so a change is seen by the next command without a restart, and
//! it lives in the guest, so it lasts exactly as long as the sandbox does.
//!
//! The file can hold credentials, so the agent only believes one it could have trusted: a
//! regular file owned by root that nobody else can write, opened without following a link.
//! Anything else is ignored whole, and so is whatever does not parse, because a malformed
//! line must not become a different variable than the one its author meant.

use std::ffi::OsString;
use std::fs::File;
use std::io::Read as _;
use std::os::unix::ffi::OsStringExt as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::Path;

use super::MAX_FIELD_BYTES;

/// Where the persisted environment lives.
pub const PATH: &str = "/etc/miosa/identity.env";
/// The most bytes of the file the agent will read.
const MAX_FILE_BYTES: u64 = 64 * 1024;
/// The most variables the agent will take from the file.
const MAX_VARIABLES: usize = 256;

/// The persisted environment, in the order the file lists it.
pub(super) fn load() -> Vec<(OsString, OsString)> {
    #[cfg(test)]
    let (path, trusted_uid) = test_support::target().unwrap_or((Path::new(PATH), 0));
    #[cfg(not(test))]
    let (path, trusted_uid) = (Path::new(PATH), 0);
    load_from(path, trusted_uid)
}

/// The persisted environment at `path`, if `trusted_uid` owns it and nobody else can write it.
pub(super) fn load_from(path: &Path, trusted_uid: u32) -> Vec<(OsString, OsString)> {
    let Ok(file) = File::options()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
    else {
        return Vec::new();
    };
    let Ok(metadata) = file.metadata() else {
        return Vec::new();
    };
    if !metadata.is_file()
        || metadata.uid() != trusted_uid
        || metadata.mode() & 0o022 != 0
        || metadata.len() > MAX_FILE_BYTES
    {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file.take(MAX_FILE_BYTES).read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    parse(&bytes)
}

/// Parses `NAME='value'` lines, skipping blanks, comments and anything malformed.
///
/// A later line for the same name replaces the earlier one in place.
pub(super) fn parse(bytes: &[u8]) -> Vec<(OsString, OsString)> {
    let mut variables: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for line in bytes.split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some(split) = line.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let (name, value) = (&line[..split], &line[split + 1..]);
        let (Some(value), true) = (unquote(value), valid_name(name)) else {
            continue;
        };
        if value.len() > MAX_FIELD_BYTES || value.contains(&0) {
            continue;
        }
        if let Some(slot) = variables.iter_mut().find(|(existing, _)| existing == name) {
            slot.1 = value;
        } else if variables.len() < MAX_VARIABLES {
            variables.push((name.to_vec(), value));
        }
    }
    variables
        .into_iter()
        .map(|(name, value)| (OsString::from_vec(name), OsString::from_vec(value)))
        .collect()
}

fn valid_name(name: &[u8]) -> bool {
    let mut bytes = name.iter();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || *first == b'_')
        && name.len() <= MAX_FIELD_BYTES
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
}

/// The value of one shell-quoted word: `'...'` with `'"'"'` standing for a quote, or bare text.
///
/// Only the two forms the control plane writes are accepted. A bare value may not hold a
/// character a shell would treat as syntax, so what the file means to the agent is what it means
/// to a shell that sources it.
fn unquote(word: &[u8]) -> Option<Vec<u8>> {
    if word.first() != Some(&b'\'') {
        return word
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_./-:@+,%".contains(byte))
            .then(|| word.to_vec());
    }
    let mut value = Vec::with_capacity(word.len());
    let mut rest = word;
    loop {
        rest = rest.strip_prefix(b"'")?;
        let end = rest.iter().position(|byte| *byte == b'\'')?;
        value.extend_from_slice(&rest[..end]);
        rest = &rest[end + 1..];
        if rest.is_empty() {
            return Some(value);
        }
        // A quote inside a quoted value is written as `'"'"'`.
        rest = rest.strip_prefix(b"\"'\"")?;
        value.push(b'\'');
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) mod test_support {
    //! Points the agent at a file of a test's own, as the unprivileged user running it.

    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, MutexGuard, PoisonError};

    static TARGET: Mutex<Option<PathBuf>> = Mutex::new(None);
    static SERIAL: Mutex<()> = Mutex::new(());

    pub(super) fn target() -> Option<(&'static Path, u32)> {
        let chosen = TARGET
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        chosen.map(|path| {
            // The test's own directory is owned by the user running it, which stands in for
            // root here. Leaked once per call, so the borrow outlives the lock.
            let uid = path
                .parent()
                .and_then(|directory| std::fs::metadata(directory).ok())
                .map_or(0, |metadata| std::os::unix::fs::MetadataExt::uid(&metadata));
            (Box::leak(path.into_boxed_path()) as &'static Path, uid)
        })
    }

    /// Holds the target for the length of one test; tests that use it run one at a time.
    pub(crate) struct Pointed {
        _serial: MutexGuard<'static, ()>,
    }

    impl Drop for Pointed {
        fn drop(&mut self) {
            *TARGET.lock().unwrap_or_else(PoisonError::into_inner) = None;
        }
    }

    pub(crate) fn point_at(path: &Path) -> Pointed {
        let serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        *TARGET.lock().unwrap_or_else(PoisonError::into_inner) = Some(path.to_path_buf());
        Pointed { _serial: serial }
    }
}
