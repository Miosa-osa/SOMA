//! What the filesystem wire form must accept, and what it must refuse.

use super::super::{HostMessage, OperationId};
use super::{
    DetailedEntry, DirectoryEntry, EntryKind, FileFailure, FileOutcome, FileRequest,
    MAX_CHUNK_BYTES, MAX_ENTRIES, MAX_PATH_BYTES,
};

mod refusals;

fn operation() -> OperationId {
    OperationId::new([7; 16]).expect("a non-zero identity")
}

fn round_trip(request: &FileRequest) {
    let encoded = HostMessage::file(operation(), request.clone())
        .encode()
        .expect("a bounded request fits one record");
    let decoded = HostMessage::decode(&encoded).expect("its own encoding decodes");
    assert_eq!(decoded, HostMessage::file(operation(), request.clone()));
}

/// Every request survives its own encoding exactly.
#[test]
fn every_request_round_trips() {
    for request in [
        FileRequest::Read {
            path: b"/workspace/main.rs".to_vec().into(),
            offset: 4096,
            length: u32::try_from(MAX_CHUNK_BYTES).expect("a bounded chunk"),
        },
        FileRequest::Write {
            path: b"/workspace/out.txt".to_vec().into(),
            offset: 0,
            create: true,
            shorten: true,
            bytes: b"hello".to_vec().into(),
        },
        FileRequest::MakeDirectory {
            path: b"/workspace/nested/dir".to_vec().into(),
            parents: true,
        },
        FileRequest::ReadDirectory {
            path: b"/workspace".to_vec().into(),
            offset: 512,
        },
        FileRequest::Exists {
            path: b"/workspace".to_vec().into(),
        },
        FileRequest::Remove {
            path: b"/workspace/tmp".to_vec().into(),
            recursive: true,
        },
        FileRequest::Create {
            path: b"/run/secrets/token".to_vec().into(),
            mode: 0o600,
        },
        FileRequest::SetMode {
            path: b"/run/secrets/token".to_vec().into(),
            mode: 0o400,
        },
        FileRequest::ReadDirectoryDetailed {
            path: b"/workspace".to_vec().into(),
            offset: 7,
        },
        FileRequest::Stat {
            path: b"/workspace/main.rs".to_vec().into(),
        },
        FileRequest::Rename {
            path: b"/etc/miosa/.identity.env.tmp".to_vec().into(),
            to: b"/etc/miosa/identity.env".to_vec().into(),
        },
    ] {
        round_trip(&request);
    }
}

/// Every outcome survives its own encoding exactly.
#[test]
fn every_outcome_round_trips() {
    use super::super::GuestMessage;
    for outcome in [
        FileOutcome::Read {
            bytes: b"contents".to_vec().into(),
            end: true,
        },
        FileOutcome::Written { bytes: 8 },
        FileOutcome::Listed {
            entries: vec![
                DirectoryEntry {
                    name: b"src".to_vec().into(),
                    kind: EntryKind::Directory,
                },
                DirectoryEntry {
                    name: b"main.rs".to_vec().into(),
                    kind: EntryKind::File,
                },
            ],
            more: false,
        },
        FileOutcome::Status {
            kind: Some(EntryKind::File),
        },
        FileOutcome::Status { kind: None },
        FileOutcome::ListedDetailed {
            entries: vec![
                DetailedEntry {
                    name: b"src".to_vec().into(),
                    kind: EntryKind::Directory,
                    size: 0,
                    mode: 0o755,
                    modified: 1_760_000_000,
                },
                DetailedEntry {
                    name: b"main.rs".to_vec().into(),
                    kind: EntryKind::File,
                    size: 1 << 33,
                    mode: 0o644,
                    modified: 0,
                },
            ],
            more: true,
        },
        FileOutcome::Stat {
            kind: EntryKind::File,
            size: 1 << 40,
            mode: 0o600,
            modified: 1_760_000_000,
        },
        FileOutcome::Done,
        FileOutcome::Failed(FileFailure::NotFound),
    ] {
        let message = GuestMessage::file_outcome(operation(), outcome.clone());
        let encoded = message.encode().expect("a bounded outcome fits one record");
        assert_eq!(GuestMessage::decode(&encoded).as_ref(), Ok(&message));
    }
}
