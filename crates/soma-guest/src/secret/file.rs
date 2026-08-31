//! One secret and the guest file it is to become.
//!
//! This is the first of the two delivery modes SOMA distinguishes: a program that genuinely
//! needs the credential itself, in a file of its own inside the guest. The second mode, where a
//! host-side mediator holds the credential and the guest only ever makes authenticated requests
//! through it, is a different mechanism and is not implemented here.
//!
//! A destination is bound to one Instance and to one launch. Nothing about it is written into
//! the Template, the Template Lock, the Generation, or the snapshot the Generation shares, so
//! two Instances of one Generation never see each other's values and a captured machine carries
//! none.

use core::fmt;

use crate::{Error, MAX_FILE_MODE};

use super::SecretValue;

/// The mode a file-delivered secret is given when the Template names none.
///
/// This is owner-read-only, and it is the same value `soma-template` records as
/// `DEFAULT_SECRET_FILE_MODE`. The two constants are stated separately because the schema crate
/// and the session crate do not depend on one another, and [`SecretFile::new`] refuses any mode
/// that is wider than the rule both of them apply.
pub const DEFAULT_SECRET_FILE_MODE: u32 = 0o400;

/// One secret value and the guest path and mode it is delivered as.
pub struct SecretFile {
    path: Box<[u8]>,
    mode: u32,
    value: SecretValue,
}

impl SecretFile {
    /// Binds one value to one absolute guest path and the mode the file is to end with.
    ///
    /// A `None` mode is the Template's own default of owner-read-only rather than whatever the
    /// guest's ambient mask would have produced, because a mask is an inherited setting and a
    /// credential's permissions must not depend on one.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSecret`] for a path this protocol will not carry and for a mode
    /// that is not owner-readable, that grants anything to the group or to others, or that
    /// exceeds [`MAX_FILE_MODE`]. A credential readable by a second account inside the guest is
    /// not a delivered credential, it is a published one.
    pub fn new(path: Vec<u8>, mode: Option<u32>, value: SecretValue) -> Result<Self, Error> {
        crate::application::check_path(&path).map_err(|_| Error::InvalidSecret)?;
        let mode = mode.unwrap_or(DEFAULT_SECRET_FILE_MODE);
        if mode > MAX_FILE_MODE || mode & 0o077 != 0 || mode & 0o400 == 0 {
            return Err(Error::InvalidSecret);
        }
        Ok(Self {
            path: path.into_boxed_slice(),
            mode,
            value,
        })
    }

    /// The absolute guest path the secret becomes.
    #[must_use]
    pub fn path(&self) -> &[u8] {
        &self.path
    }

    /// The mode the delivered file ends with.
    #[must_use]
    pub const fn mode(&self) -> u32 {
        self.mode
    }

    /// The value, for the one crate-internal step that puts it on the session.
    pub(crate) const fn value(&self) -> &SecretValue {
        &self.value
    }

    /// The directory the destination lives in, when it is not the guest's root.
    ///
    /// A destination whose parent is the root needs no directory made, and a caller that asked
    /// for one would be asking the guest to create a path that is always already there.
    pub(crate) fn parent(&self) -> Option<&[u8]> {
        let separator = self.path.iter().rposition(|byte| *byte == b'/')?;
        (separator > 0).then(|| &self.path[..separator])
    }
}

impl fmt::Debug for SecretFile {
    /// Names the destination length and the mode, and never the path or the value.
    ///
    /// A destination path is tenant data for the same reason every other guest path on this
    /// protocol is, and the value is the thing this whole module exists to keep unprintable.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "SecretFile {{ path: {} bytes, mode: 0o{:o} }}",
            self.path.len(),
            self.mode
        )
    }
}
