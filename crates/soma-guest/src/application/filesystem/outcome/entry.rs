//! What one path is, and the two shapes one directory entry takes.

use core::fmt;

use crate::Error;

/// What one path is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EntryKind {
    /// A regular file.
    File = 1,
    /// A directory.
    Directory = 2,
    /// Anything else the guest reports, including a symbolic link or a device.
    Other = 3,
}

impl EntryKind {
    /// Decodes one entry kind.
    pub(super) fn parse(value: u8) -> Result<Self, Error> {
        match value {
            1 => Ok(Self::File),
            2 => Ok(Self::Directory),
            3 => Ok(Self::Other),
            _ => Err(Error::ApplicationMessageRejected),
        }
    }
}

/// One directory entry: its name within the directory, never a path.
#[derive(Clone, Eq, PartialEq)]
pub struct DirectoryEntry {
    /// The entry's own name, with no separator in it.
    pub name: Box<[u8]>,
    /// What the entry is.
    pub kind: EntryKind,
}

impl fmt::Debug for DirectoryEntry {
    /// Names the kind and the name length, never the name, which is tenant data.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "DirectoryEntry {{ kind: {:?}, name: {} bytes }}",
            self.kind,
            self.name.len()
        )
    }
}

/// One directory entry with the facts a listing needs without a request per entry.
#[derive(Clone, Eq, PartialEq)]
pub struct DetailedEntry {
    /// The entry's own name, with no separator in it.
    pub name: Box<[u8]>,
    /// What the entry is.
    pub kind: EntryKind,
    /// Size in bytes; zero for anything that is not a regular file.
    pub size: u64,
    /// Permission bits, within [`super::super::MAX_FILE_MODE`].
    pub mode: u32,
    /// Modification time in whole seconds since the Unix epoch, zero when unknown.
    pub modified: u64,
}

impl DetailedEntry {
    /// Bytes this entry takes in an encoded listing.
    #[must_use]
    pub const fn encoded_len(&self) -> usize {
        1 + 2 + self.name.len() + 8 + 4 + 8
    }
}

impl fmt::Debug for DetailedEntry {
    /// Names the kind and the name length, never the name, which is tenant data.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "DetailedEntry {{ kind: {:?}, name: {} bytes }}",
            self.kind,
            self.name.len()
        )
    }
}
