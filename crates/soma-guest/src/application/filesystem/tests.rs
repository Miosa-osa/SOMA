//! What the filesystem wire form must accept, and what it must refuse.

use super::super::{HostMessage, OperationId};
use super::{
    DirectoryEntry, EntryKind, FileFailure, FileOutcome, FileRequest, MAX_CHUNK_BYTES,
    MAX_PATH_BYTES,
};

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
        FileOutcome::Done,
        FileOutcome::Failed(FileFailure::NotFound),
    ] {
        let message = GuestMessage::file_outcome(operation(), outcome.clone());
        let encoded = message.encode().expect("a bounded outcome fits one record");
        assert_eq!(GuestMessage::decode(&encoded).as_ref(), Ok(&message));
    }
}

/// A path this protocol will not carry is refused at the decoder, not at the guest.
#[test]
fn an_inadmissible_path_is_refused() {
    for path in [
        b"".to_vec(),                   // empty
        b"relative/path".to_vec(),      // not absolute
        b"/has\0interior".to_vec(),     // interior nul
        vec![b'/'; MAX_PATH_BYTES + 1], // longer than the bound
    ] {
        let request = FileRequest::Exists { path: path.into() };
        let encoded = HostMessage::file(operation(), request).encode();
        // Either the encoder refused it, or the decoder must.
        if let Ok(bytes) = encoded {
            assert!(
                HostMessage::decode(&bytes).is_err(),
                "an inadmissible path decoded"
            );
        }
    }
}

/// A directory entry naming something outside its own directory is refused.
///
/// A listing that could return `..` or a path with a separator would describe entries the
/// caller did not ask about, so the decoder rejects the shape rather than trusting the guest.
#[test]
fn an_entry_name_that_is_not_one_component_is_refused() {
    use super::super::GuestMessage;
    for name in [b"..\0".to_vec(), b"nested/name".to_vec(), b"".to_vec()] {
        let outcome = FileOutcome::Listed {
            entries: vec![DirectoryEntry {
                name: name.into(),
                kind: EntryKind::File,
            }],
            more: false,
        };
        let encoded = GuestMessage::file_outcome(operation(), outcome)
            .encode()
            .expect("encodes");
        assert!(
            GuestMessage::decode(&encoded).is_err(),
            "an entry name that is not one component decoded"
        );
    }
}

/// A boolean is exactly zero or one, so one message has exactly one encoding.
#[test]
fn a_boolean_that_is_neither_zero_nor_one_is_refused() {
    let request = FileRequest::Remove {
        path: b"/workspace".to_vec().into(),
        recursive: false,
    };
    let mut encoded = HostMessage::file(operation(), request)
        .encode()
        .expect("encodes");
    let last = encoded.len() - 1;
    encoded[last] = 2;
    assert!(
        HostMessage::decode(&encoded).is_err(),
        "a boolean of two decoded"
    );
}

/// A trailing byte is a different message, so it is refused rather than ignored.
#[test]
fn a_trailing_byte_is_refused() {
    let request = FileRequest::Exists {
        path: b"/workspace".to_vec().into(),
    };
    let mut encoded = HostMessage::file(operation(), request)
        .encode()
        .expect("encodes");
    encoded.push(0);
    assert!(
        HostMessage::decode(&encoded).is_err(),
        "a trailing byte decoded"
    );
}

/// A mode outside the permission bits is a message this protocol does not carry.
#[test]
fn a_mode_above_the_permission_bits_is_refused() {
    for mode in [0o1000_u32, 0o4755, u32::MAX] {
        let request = FileRequest::SetMode {
            path: b"/workspace/file".to_vec().into(),
            mode,
        };
        let encoded = HostMessage::file(operation(), request)
            .encode()
            .expect("an encoder writes what it is given");
        assert!(
            HostMessage::decode(&encoded).is_err(),
            "mode 0o{mode:o} decoded"
        );
    }
}

/// Neither a path nor file bytes may reach a log through a formatter.
#[test]
fn debug_reports_shapes_and_never_content() {
    let request = FileRequest::Write {
        path: b"/workspace/secret.env".to_vec().into(),
        offset: 0,
        create: true,
        shorten: false,
        bytes: b"TOKEN=hunter2".to_vec().into(),
    };
    let rendered = format!("{request:?}");
    assert!(!rendered.contains("secret.env"), "{rendered}");
    assert!(!rendered.contains("hunter2"), "{rendered}");
    assert!(rendered.contains("21 bytes"), "{rendered}");

    let outcome = FileOutcome::Read {
        bytes: b"TOKEN=hunter2".to_vec().into(),
        end: true,
    };
    let rendered = format!("{outcome:?}");
    assert!(!rendered.contains("hunter2"), "{rendered}");
}
