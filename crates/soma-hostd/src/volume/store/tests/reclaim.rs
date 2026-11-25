//! A destroyed volume owes nothing, so nothing of it is kept (SOMA#246).

use super::*;
use crate::volume::record::{VolumeRecord, VolumeRecordKind};

fn record_directory(directory: &tempfile::TempDir, volume_id: VolumeId) -> std::path::PathBuf {
    directory.path().join("records").join(volume_id.hex())
}

fn key_directory(directory: &tempfile::TempDir, volume_id: VolumeId) -> std::path::PathBuf {
    directory
        .path()
        .join("records")
        .join("keys")
        .join(volume_id.hex())
}

#[test]
fn a_destroy_leaves_no_records_and_no_key_directory() {
    let directory = tempfile::tempdir().expect("temp dir");
    let volumes = with_repairs(&directory, Box::new(CleanRepair));
    volumes.attach(&attach(1, 9, 1, 10)).expect("attaches");
    volumes.destroy(volume(1)).expect("destroys");

    assert!(
        !record_directory(&directory, volume(1)).exists(),
        "a destroyed volume's records were kept"
    );
    assert!(
        !key_directory(&directory, volume(1)).exists(),
        "a destroyed volume's key directory was kept"
    );
}

/// A host that stopped between the end of a destroy and the removal of its records removes them
/// when it opens again; records of a volume whose destroy never ended are kept.
#[test]
fn opening_removes_the_records_of_a_volume_whose_destroy_ended() {
    let directory = tempfile::tempdir().expect("temp dir");
    for (volume_id, last) in [
        (volume(2), VolumeRecordKind::DestroyEnd),
        (volume(3), VolumeRecordKind::DestroyBegin),
    ] {
        let records = record_directory(&directory, volume_id);
        std::fs::create_dir_all(&records).expect("record directory");
        std::fs::create_dir_all(key_directory(&directory, volume_id)).expect("key directory");
        for (sequence, kind) in [
            VolumeRecordKind::CreateBegin,
            VolumeRecordKind::CreateEnd,
            last,
        ]
        .into_iter()
        .enumerate()
        {
            std::fs::write(
                records.join(format!("{sequence:020}")),
                VolumeRecord::new(kind, volume_id).encode(),
            )
            .expect("record");
        }
    }

    let _volumes = with_repairs(&directory, Box::new(CleanRepair));

    assert!(!record_directory(&directory, volume(2)).exists());
    assert!(!key_directory(&directory, volume(2)).exists());
    assert!(
        record_directory(&directory, volume(3)).exists(),
        "a destroy that never ended still owes its records"
    );
}
