//! Bringing a file into being at a chosen mode, and changing the mode of one that exists.
//!
//! A caller that asks for a mode means that mode. The kernel applies the process umask to the
//! mode an `open` requests, so a file created with the mode alone can only ever come out narrower
//! than asked for, and the umask is an inherited setting no caller of this protocol can see. The
//! creation therefore sets the mode explicitly on the descriptor it just made, which is not a
//! widening of anything: the file was created exclusively, so between the two calls only this
//! process holds it, and the mode it briefly had was at most the mode that was asked for.

use std::fs::{OpenOptions, Permissions, set_permissions};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use soma_guest::FileOutcome;

use super::failure;

/// Creates a new file at exactly `mode`, refusing a path that already exists.
///
/// The exclusive create is the whole point: a caller choosing the mode of a file is choosing to
/// own it, and silently adopting whatever was already at the path would hand it a file whose
/// mode, owner, and link target someone else chose.
pub(super) fn create(path: &Path, mode: u32) -> FileOutcome {
    let file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(error) => return failure::failed(&error),
    };
    failure::done(file.set_permissions(Permissions::from_mode(mode)))
}

/// Sets the permission bits of a path that already exists.
pub(super) fn set(path: &Path, mode: u32) -> FileOutcome {
    failure::done(set_permissions(path, Permissions::from_mode(mode)))
}
