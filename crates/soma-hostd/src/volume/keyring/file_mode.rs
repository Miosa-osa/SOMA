//! The mode a key file is created with.

use std::fs::OpenOptions;

/// Creates files `0600`, readable and writable by their owner alone.
#[cfg(unix)]
pub(super) fn owner_only(options: &mut OpenOptions) -> &mut OpenOptions {
    use std::os::unix::fs::OpenOptionsExt as _;
    options.mode(0o600)
}

/// Has no mode bits to set off unix.
///
/// A key is still never written to an unencrypted root there: off Linux the store certifies no
/// mount as encrypted, so it refuses every volume before a key is asked for.
#[cfg(not(unix))]
pub(super) const fn owner_only(options: &mut OpenOptions) -> &mut OpenOptions {
    options
}
