//! What the filesystem wire form must refuse.

use crate::GuestMessage;

use super::*;

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

/// A rename names two admissible paths; either one being inadmissible refuses the body.
#[test]
fn a_rename_with_an_inadmissible_path_is_refused() {
    for (from, to) in [
        (&b"relative"[..], &b"/ok"[..]),
        (&b"/ok"[..], &b"relative"[..]),
        (&b"/ok"[..], &b""[..]),
        (&b"/ok"[..], &b"/a\0b"[..]),
    ] {
        let mut body = vec![10u8];
        for path in [from, to] {
            body.extend_from_slice(&u16::try_from(path.len()).unwrap().to_be_bytes());
            body.extend_from_slice(path);
        }
        assert!(FileRequest::decode_body(&body).is_err());
    }
}

/// A stat answer carrying a mode outside the permission bits, or a trailing byte, is refused.
#[test]
fn a_hostile_stat_answer_is_refused() {
    let good = FileOutcome::Stat {
        kind: EntryKind::Directory,
        size: 0,
        mode: 0o755,
        modified: 0,
    }
    .encode_body();
    assert!(FileOutcome::decode_body(&good).is_ok());

    let mut high_mode = good.clone();
    high_mode[10..14].copy_from_slice(&0o4755u32.to_be_bytes());
    assert!(FileOutcome::decode_body(&high_mode).is_err());

    let mut trailing = good.clone();
    trailing.push(0);
    assert!(FileOutcome::decode_body(&trailing).is_err());

    let mut bad_kind = good;
    bad_kind[1] = 9;
    assert!(FileOutcome::decode_body(&bad_kind).is_err());
    assert!(FileOutcome::decode_body(&bad_kind[..5]).is_err());
}

/// A detailed listing is refused when an entry could not be a child of the listed directory,
/// carries a mode the protocol does not, or claims more entries than the bound.
#[test]
fn a_hostile_detailed_listing_is_refused() {
    let entry = |name: &[u8], mode: u32| DetailedEntry {
        name: name.to_vec().into(),
        kind: EntryKind::File,
        size: 1,
        mode,
        modified: 1,
    };
    for (name, mode) in [
        (&b"a/b"[..], 0o644),
        (&b""[..], 0o644),
        (&b"a\0b"[..], 0o644),
        (&b"ok"[..], 0o4755),
    ] {
        let body = FileOutcome::ListedDetailed {
            entries: vec![entry(name, mode)],
            more: false,
        }
        .encode_body();
        assert!(
            FileOutcome::decode_body(&body).is_err(),
            "{name:?} {mode:o}"
        );
    }

    let mut over = FileOutcome::ListedDetailed {
        entries: Vec::new(),
        more: false,
    }
    .encode_body();
    over[2..6].copy_from_slice(&u32::try_from(MAX_ENTRIES + 1).expect("count").to_be_bytes());
    assert!(FileOutcome::decode_body(&over).is_err());

    let mut trailing = FileOutcome::ListedDetailed {
        entries: vec![entry(b"ok", 0o644)],
        more: false,
    }
    .encode_body();
    trailing.push(0);
    assert!(FileOutcome::decode_body(&trailing).is_err());
}
