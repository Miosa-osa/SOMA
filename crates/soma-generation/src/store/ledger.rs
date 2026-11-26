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
///
/// The type is shared with checkpoint memory attestation so both records mean the same thing.
pub(super) use soma_kvm::snapshot::FileStamp as Stamp;

/// The stamp of an open blob, or `None` where the platform cannot say.
#[cfg(unix)]
#[allow(
    clippy::unnecessary_wraps,
    reason = "platforms that cannot stamp a blob return None from the other definition"
)]
pub(super) fn stamp_of(metadata: &cap_std::fs::Metadata) -> Option<Stamp> {
    use cap_std::fs::MetadataExt as _;
    Some(Stamp::new(
        metadata.dev(),
        metadata.ino(),
        metadata.size(),
        (metadata.mtime(), metadata.mtime_nsec()),
        (metadata.ctime(), metadata.ctime_nsec()),
    ))
}

#[cfg(not(unix))]
pub(super) fn stamp_of(_metadata: &cap_std::fs::Metadata) -> Option<Stamp> {
    None
}

fn encode(name: &str, stamp: Stamp) -> String {
    format!("{name} {}\n", stamp.encode())
}

fn decode(line: &str) -> Option<(String, Stamp)> {
    let mut fields = line.split(' ');
    let name = fields.next()?.to_owned();
    Some((name, Stamp::decode(&mut fields)?))
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
            .map(|text| text.lines().filter_map(decode).collect())
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
            .map(|(known, stamp)| encode(known, *stamp))
            .collect();
        text.sort();
        if version.write(STAGING, text.concat()).is_ok() {
            let _saved = version.rename(STAGING, version, LEDGER);
        }
    }
}
