//! What a destroyed volume leaves behind, and its removal (SOMA#246).
//!
//! A destroy writes `DestroyEnd` once the file and its key are gone. From then on the volume owes
//! nothing: the store rebuilds its state from sidecars, and the only reader of the write-ahead
//! records addresses a live volume. So its records and its key's directory go with it, and a
//! store that opens after a stop between the two removes them then.

use std::{collections::BTreeMap, fs, path::Path};

use crate::VolumeId;

use super::super::keyring::KEYS_DIR;
use super::super::record::{VolumeRecord, VolumeRecordKind};
use super::super::sidecar::VolumeSidecar;
use super::DirectoryVolumes;

impl DirectoryVolumes {
    /// Removes the records and the key directory of a volume whose destroy ended.
    ///
    /// A failure leaves them for the next open to remove, so it is not an error of the destroy.
    pub(super) fn forget(&self, volume: VolumeId) {
        forget_records(&self.records, volume);
    }
}

/// Removes the records of every volume with no sidecar whose last record is `DestroyEnd`.
///
/// A volume whose last record is anything else stopped mid-operation and keeps its records.
pub(super) fn forget_ended(records: &Path, live: &BTreeMap<VolumeId, VolumeSidecar>) {
    let Ok(entries) = fs::read_dir(records) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(volume) = entry
            .file_name()
            .to_str()
            .and_then(|name| VolumeId::from_hex(name).ok())
        else {
            continue;
        };
        if !live.contains_key(&volume)
            && last_record(&entry.path()) == Some(VolumeRecordKind::DestroyEnd)
        {
            forget_records(records, volume);
        }
    }
}

fn last_record(directory: &Path) -> Option<VolumeRecordKind> {
    let latest = fs::read_dir(directory)
        .ok()?
        .flatten()
        .max_by_key(fs::DirEntry::file_name)?;
    let record = VolumeRecord::decode(&fs::read(latest.path()).ok()?)?;
    Some(record.kind)
}

fn forget_records(records: &Path, volume: VolumeId) {
    let _ = fs::remove_dir_all(records.join(volume.hex()));
    // Only an empty key directory is removed: a key file still in it is a key nobody destroyed.
    let _ = fs::remove_dir(records.join(KEYS_DIR).join(volume.hex()));
}
