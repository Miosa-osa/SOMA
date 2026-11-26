use std::{fs::File, io::Write as _};

use soma_kvm::snapshot::{ArtifactAttestation, Digest, FileStamp};

use super::super::build::Recovery;
use crate::backend::kvm::checkpoint::RestoreArtifacts;

fn file_with(bytes: &[u8]) -> File {
    let mut file = tempfile::tempfile().expect("scratch file");
    file.write_all(bytes).expect("write");
    file
}

fn stamp(file: &File) -> FileStamp {
    FileStamp::of(&file.metadata().expect("stat"))
}

/// A published checkpoint with a saved overlay, as `restore_artifacts` opens one.
fn checkpoint(overlay_recorded: bool) -> RestoreArtifacts {
    let overlay = file_with(&[5_u8; 8192]);
    let overlay_attestation =
        overlay_recorded.then(|| ArtifactAttestation::new(Digest::of(b"overlay"), stamp(&overlay)));
    RestoreArtifacts {
        state: file_with(b"state"),
        memory: file_with(&[3_u8; 8192]),
        overlay: Some(overlay),
        digest: Digest::of(b"state"),
        cpu_template: Digest::of(b"cpu"),
        volume: None,
        memory_attestation: Some(ArtifactAttestation::new(
            Digest::of(b"memory"),
            FileStamp::new(1, 2, 8192, (3, 4), (5, 6)),
        )),
        overlay_attestation,
    }
}

#[test]
fn each_attestation_goes_to_its_own_input() {
    let checkpoint = checkpoint(true);
    let head = file_with(&[5_u8; 8192]);
    let root = ArtifactAttestation::new(Digest::of(b"root"), FileStamp::new(7, 8, 9, (1, 1), (2, 2)));
    let wired = Recovery::attestations(&checkpoint, Some(&head), Some(root), true);
    assert_eq!(wired.memory, checkpoint.memory_attestation);
    assert_eq!(wired.root, Some(root));
    let head_record = wired.head.expect("vouched head");
    // The head carries the saved overlay's digest at the head's own stamp, never the overlay's.
    assert_eq!(head_record.digest(), Digest::of(b"overlay"));
    assert_eq!(head_record.stamp(), stamp(&head));
    assert_ne!(Some(head_record), checkpoint.overlay_attestation);
}

#[test]
fn a_head_gets_no_attestation_without_a_record_a_private_root_or_a_head() {
    let head = file_with(&[5_u8; 8192]);
    let unrecorded = checkpoint(false);
    assert_eq!(Recovery::attestations(&unrecorded, Some(&head), None, true).head, None);
    let recorded = checkpoint(true);
    assert_eq!(Recovery::attestations(&recorded, Some(&head), None, false).head, None);
    assert_eq!(Recovery::attestations(&recorded, None, None, true).head, None);
    // A checkpoint without a saved overlay, like a Generation without writable storage, has no
    // head to vouch for whatever file is offered.
    let mut no_overlay = checkpoint(true);
    no_overlay.overlay = None;
    assert_eq!(Recovery::attestations(&no_overlay, Some(&head), None, true).head, None);
}
