//! A record of which blobs an earlier admission already hashed, so a restart does not hash them again.
//!
//! Admitting a Generation reads every launch artifact end to end, which is tens of seconds for a
//! 40 GiB writable template and is paid by every start of a hosted service. The bytes did not change
//! between starts, and the kernel says so: a blob is identified by its device, inode, size, and
//! modification and change times, and the change time moves on any write, resize, rename-over, or
//! permission change and cannot be set from user space. A blob whose stamp equals the one recorded when
//! its content was last hashed is the same bytes, so it is admitted without a read. Anything else,
//! including a ledger that is absent, unreadable, malformed, or written for another filesystem, hashes
//! the blob as before.
//!
//! The ledger lives beside the blobs in the store's own `v1` directory and is trusted exactly as far
//! as the store is: whoever can write the ledger can already write the blobs.

use std::{collections::HashMap, sync::Mutex};

use cap_std::fs::Dir;

const LEDGER: &str = "admitted-ledger";
const STAGING: &str = "admitted-ledger.tmp";

/// What the kernel reports about one blob, enough to tell it from any other state of the same name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Stamp {
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl Stamp {
    /// The stamp of an open blob, or `None` where the platform cannot say.
    #[cfg(unix)]
    #[allow(
        clippy::unnecessary_wraps,
        reason = "platforms that cannot stamp a blob return None from the other definition"
    )]
    pub(super) fn of(metadata: &cap_std::fs::Metadata) -> Option<Self> {
        use cap_std::fs::MetadataExt as _;
        Some(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.size(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }

    #[cfg(not(unix))]
    pub(super) fn of(_metadata: &cap_std::fs::Metadata) -> Option<Self> {
        None
    }

    fn encode(self, name: &str) -> String {
        format!(
            "{name} {} {} {} {} {} {} {}\n",
            self.device,
            self.inode,
            self.size,
            self.modified.0,
            self.modified.1,
            self.changed.0,
            self.changed.1
        )
    }

    fn decode(line: &str) -> Option<(String, Self)> {
        let mut fields = line.split(' ');
        let name = fields.next()?.to_owned();
        let mut number = || fields.next()?.parse::<i64>().ok();
        let device = u64::try_from(number()?).ok()?;
        let inode = u64::try_from(number()?).ok()?;
        let size = u64::try_from(number()?).ok()?;
        let modified = (number()?, number()?);
        let changed = (number()?, number()?);
        Some((
            name,
            Self {
                device,
                inode,
                size,
                modified,
                changed,
            },
        ))
    }
}

/// The recorded stamps of blobs whose content has been hashed.
pub(super) struct Ledger {
    entries: Mutex<HashMap<String, Stamp>>,
}

impl Ledger {
    /// Reads the ledger of a store; a missing or damaged one is an empty ledger.
    pub(super) fn load(version: &Dir) -> Self {
        let entries = version
            .read_to_string(LEDGER)
            .map(|text| text.lines().filter_map(Stamp::decode).collect())
            .unwrap_or_default();
        Self {
            entries: Mutex::new(entries),
        }
    }

    /// Whether `name` was hashed while it had exactly this stamp.
    pub(super) fn knows(&self, name: &str, stamp: Stamp) -> bool {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(name)
            == Some(&stamp)
    }

    /// Notes that `name` was just hashed at this stamp, and saves the ledger if the store allows it.
    ///
    /// A store that cannot be written (read-only, full) keeps working: the next start hashes again.
    pub(super) fn record(&self, version: &Dir, name: &str, stamp: Stamp) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.insert(name.to_owned(), stamp);
        let mut text: Vec<String> = entries
            .iter()
            .map(|(known, stamp)| stamp.encode(known))
            .collect();
        text.sort();
        if version.write(STAGING, text.concat()).is_ok() {
            let _saved = version.rename(STAGING, version, LEDGER);
        }
    }
}
