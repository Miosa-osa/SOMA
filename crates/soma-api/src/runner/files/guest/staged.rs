//! A file being written beside its target, published by one rename once it is whole.
//!
//! A JSON write fills its sibling in one call; an upload fills it as the body arrives, one
//! transfer at a time, with the network between them. Both publish the same way: the sibling is
//! given its mode and renamed over the target, so a reader sees the old file or the new one, and
//! a write that stops part way leaves the target as it was once its sibling is discarded.

use super::{
    GuestFiles, Stop, make_directory, remove, rename, set_mode,
    transfer::{temporary_name, write_ranges},
};
use crate::runner::files::path::GuestPath;

/// One sibling being filled.
#[derive(Clone, Debug)]
pub(crate) struct Staged {
    temporary: GuestPath,
    target: GuestPath,
    written: u64,
    /// Whether the sibling exists yet; it is created by the first write into it.
    created: bool,
}

impl Staged {
    /// Makes the target's directory and names the sibling the bytes will fill.
    pub(crate) fn begin(guest: &mut dyn GuestFiles, target: GuestPath) -> Result<Self, Stop> {
        make_directory(guest, &target.parent())?;
        Ok(Self {
            temporary: target.sibling(&temporary_name()),
            target,
            written: 0,
            created: false,
        })
    }

    /// Adds `bytes` to the end of the sibling.
    pub(crate) fn append(&mut self, guest: &mut dyn GuestFiles, bytes: &[u8]) -> Result<(), Stop> {
        if bytes.is_empty() {
            return Ok(());
        }
        write_ranges(guest, &self.temporary, self.written, bytes, !self.created)?;
        self.created = true;
        self.written = self
            .written
            .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        Ok(())
    }

    /// Gives the sibling `mode` and renames it over the target; returns the file's size.
    pub(crate) fn publish(&mut self, guest: &mut dyn GuestFiles, mode: u32) -> Result<u64, Stop> {
        if !self.created {
            // An empty file is still a file: the sibling is created with no bytes in it.
            write_ranges(guest, &self.temporary, 0, &[], true)?;
            self.created = true;
        }
        set_mode(guest, &self.temporary, mode)?;
        rename(guest, &self.temporary, &self.target)?;
        Ok(self.written)
    }

    /// Removes the sibling, as best it can: its removal failing changes nothing about why the
    /// write it belonged to did.
    pub(crate) fn discard(&self, guest: &mut dyn GuestFiles) {
        if self.created {
            let _removed = remove(guest, &self.temporary, false);
        }
    }
}
