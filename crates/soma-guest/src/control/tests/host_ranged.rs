use std::thread;

use crate::{
    FileFailure, FileOutcome, FileRequest, GUEST_API_PROBE_PREFIX, GuestApi, GuestMessage,
    HostMessage, MAX_CHUNK_BYTES,
};

use super::super::{RangeRead, RangeWrite, WholeFileWrite};
use super::host_file::repaired_host;

/// Answers the one probe read the host sends with `answer`, and returns the path it read.
fn serve_probe(nonce: &'static [u8], answer: FileOutcome) -> (GuestApi, Vec<u8>) {
    let (host, mut raw, observed) = repaired_host();
    let host_thread = thread::spawn(move || host.probe_guest_api(nonce));
    let HostMessage::File { operation, request } = raw.receive() else {
        panic!("the host sends a file request");
    };
    let FileRequest::Read { path, .. } = request else {
        panic!("a probe is an ordinary read");
    };
    raw.send(GuestMessage::file_outcome(operation, answer));
    let (_host, api) = host_thread.join().expect("host thread").expect("probe");
    assert_eq!(observed.poison(), 0);
    (api, path.into_vec())
}

fn text(bytes: &[u8]) -> FileOutcome {
    FileOutcome::Read {
        bytes: bytes.into(),
        end: true,
    }
}

#[test]
fn a_new_guest_echoing_the_nonce_is_level_two() {
    let (api, path) = serve_probe(b"abc123", text(b"2 abc123\n"));
    assert_eq!(api, GuestApi::Level(2));
    assert!(api.has_ranged_files());
    assert_eq!(path, [GUEST_API_PROBE_PREFIX, b"abc123"].concat());
}

#[test]
fn an_old_guest_that_fails_the_read_is_legacy() {
    let (api, _) = serve_probe(b"abc123", FileOutcome::Failed(FileFailure::NotFound));
    assert_eq!(api, GuestApi::Legacy);
    assert!(!api.has_ranged_files());
}

#[test]
fn a_file_a_tenant_planted_at_the_probe_path_never_passes_for_the_answer() {
    for planted in [
        &b"2"[..],
        b"2\n",
        b"2 \n",
        b"2 other\n",
        b"2 abc123",
        b"22 abc123\n",
        b"x abc123\n",
        b"2 abc123\nextra",
        b"",
    ] {
        let (api, _) = serve_probe(b"abc123", text(planted));
        assert_eq!(api, GuestApi::Legacy, "{planted:?}");
    }
}

#[test]
fn an_unusable_nonce_is_legacy_and_sends_nothing() {
    for nonce in [&b""[..], b"a/b", b"a b"] {
        let (host, _raw, observed) = repaired_host();
        let (_host, api) = host.probe_guest_api(nonce).expect("no traffic, no failure");
        assert_eq!(api, GuestApi::Legacy);
        assert_eq!(observed.poison(), 0);
    }
}

#[test]
fn a_range_read_asks_for_exactly_the_range_and_stops_at_its_length() {
    let length = MAX_CHUNK_BYTES + 5;
    let (host, mut raw, observed) = repaired_host();
    let host_thread =
        thread::spawn(move || host.read_range(b"/tmp/file", 1000, length, MAX_CHUNK_BYTES * 2));
    let mut seen = Vec::new();
    let mut served = 0;
    while served < length {
        let HostMessage::File { operation, request } = raw.receive() else {
            panic!("the host sends a file request");
        };
        let FileRequest::Read {
            offset,
            length: wanted,
            ..
        } = request
        else {
            panic!("a range read asks for reads");
        };
        seen.push(offset);
        let wanted = usize::try_from(wanted).expect("a bounded length");
        raw.send(GuestMessage::file_outcome(
            operation,
            FileOutcome::Read {
                bytes: vec![7; wanted].into(),
                end: false,
            },
        ));
        served += wanted;
    }
    let (_host, outcome) = host_thread.join().expect("host thread").expect("read");
    assert_eq!(
        outcome,
        RangeRead::Bytes {
            bytes: vec![7; length],
            end: false
        }
    );
    assert_eq!(
        seen,
        vec![1000, 1000 + u64::try_from(MAX_CHUNK_BYTES).expect("offset")]
    );
    assert_eq!(observed.poison(), 0);
}

#[test]
fn a_range_over_the_bound_is_refused_before_anything_is_sent() {
    let (host, _raw, observed) = repaired_host();
    let (_host, outcome) = host.read_range(b"/f", 0, 11, 10).expect("refusal");
    assert_eq!(outcome, RangeRead::TooLarge);
    let (host, _raw, _observed) = repaired_host();
    let (_host, outcome) = host
        .write_range(
            RangeWrite {
                path: b"/f",
                offset: 0,
                create: true,
                shorten: false,
                bytes: &[0; 11],
            },
            10,
        )
        .expect("refusal");
    assert_eq!(outcome, WholeFileWrite::TooLarge);
    assert_eq!(observed.poison(), 0);
}

#[test]
fn a_guest_returning_more_than_asked_poisons_the_range_read() {
    let (host, mut raw, observed) = repaired_host();
    let host_thread = thread::spawn(move || host.read_range(b"/f", 0, 4, 8));
    let HostMessage::File { operation, .. } = raw.receive() else {
        panic!("the host sends a file request");
    };
    raw.send(GuestMessage::file_outcome(
        operation,
        FileOutcome::Read {
            bytes: vec![0; 5].into(),
            end: false,
        },
    ));
    assert!(host_thread.join().expect("host thread").is_err());
    assert_eq!(observed.poison(), 1);
}

#[test]
fn a_range_write_creates_only_first_and_shortens_only_last_and_only_when_asked() {
    for shorten in [false, true] {
        let total = MAX_CHUNK_BYTES + 3;
        let (host, mut raw, observed) = repaired_host();
        let host_thread = thread::spawn(move || {
            host.write_range(
                RangeWrite {
                    path: b"/f",
                    offset: 40,
                    create: true,
                    shorten,
                    bytes: &vec![1; total],
                },
                total,
            )
        });
        let mut records = Vec::new();
        loop {
            let HostMessage::File { operation, request } = raw.receive() else {
                panic!("the host sends a file request");
            };
            let FileRequest::Write {
                offset,
                create,
                shorten: cut,
                bytes,
                ..
            } = request
            else {
                panic!("a range write asks for writes");
            };
            records.push((offset, create, cut, bytes.len()));
            raw.send(GuestMessage::file_outcome(
                operation,
                FileOutcome::Written {
                    bytes: u64::try_from(bytes.len()).expect("count"),
                },
            ));
            if records.len() == 2 {
                break;
            }
        }
        let (_host, outcome) = host_thread.join().expect("host thread").expect("write");
        assert_eq!(outcome, WholeFileWrite::Written);
        let second = 40 + u64::try_from(MAX_CHUNK_BYTES).expect("offset");
        assert_eq!(
            records,
            vec![
                (40, true, false, MAX_CHUNK_BYTES),
                (second, false, shorten, 3)
            ]
        );
        assert_eq!(observed.poison(), 0);
    }
}

/// Plays a session through the host's gating sequence: ensure the level, then send a level 2
/// request only when the level allows it. Returns what the guest saw, in order.
fn gated_session(
    guest_answer: Option<&'static [u8]>,
    requests: usize,
    expected: usize,
) -> (Vec<String>, usize) {
    let (host, mut raw, observed) = repaired_host();
    let host_thread = thread::spawn(move || {
        let mut cache = None;
        let mut host = host;
        let mut refused = 0;
        for _ in 0..requests {
            let (next, api) = host
                .ensure_guest_api(&mut cache, || b"feedbeef".to_vec())
                .expect("gating");
            host = next;
            if api.has_ranged_files() {
                let (next, _) = host
                    .file(FileRequest::Stat {
                        path: b"/f".as_slice().into(),
                    })
                    .expect("stat");
                host = next;
            } else {
                refused += 1;
            }
        }
        refused
    });
    let mut seen = Vec::new();
    for _ in 0..expected {
        let message = raw.receive();
        let HostMessage::File { operation, request } = message else {
            panic!("the host sends file requests only");
        };
        match request {
            FileRequest::Read { path, .. } => {
                seen.push(format!("read {}", String::from_utf8_lossy(&path)));
                let outcome =
                    guest_answer.map_or(FileOutcome::Failed(FileFailure::NotFound), |bytes| {
                        FileOutcome::Read {
                            bytes: bytes.into(),
                            end: true,
                        }
                    });
                raw.send(GuestMessage::file_outcome(operation, outcome));
            }
            FileRequest::Stat { .. } => {
                seen.push("stat".to_owned());
                raw.send(GuestMessage::file_outcome(
                    operation,
                    FileOutcome::Stat {
                        kind: crate::EntryKind::File,
                        size: 1,
                        mode: 0o600,
                        modified: 0,
                    },
                ));
            }
            other => panic!("unexpected request {other:?}"),
        }
    }
    // Any frame beyond the expected ones would leave the host waiting for an answer that never
    // comes, so a host still running here sent something it should not have.
    for _ in 0..200 {
        if host_thread.is_finished() {
            break;
        }
        thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        host_thread.is_finished(),
        "the host sent an unexpected frame"
    );
    let refused = host_thread.join().expect("host thread");
    assert_eq!(observed.poison(), 0);
    (seen, refused)
}

#[test]
fn a_level_one_guest_sees_one_ordinary_read_and_never_a_level_two_tag() {
    let (seen, refused) = gated_session(None, 3, 1);
    assert_eq!(seen, vec!["read /.soma/guest-api/feedbeef".to_owned()]);
    assert_eq!(refused, 3);
}

#[test]
fn a_level_two_guest_is_probed_once_and_then_answers_every_request() {
    let (seen, refused) = gated_session(Some(b"2 feedbeef\n"), 3, 4);
    assert_eq!(
        seen,
        vec![
            "read /.soma/guest-api/feedbeef".to_owned(),
            "stat".to_owned(),
            "stat".to_owned(),
            "stat".to_owned()
        ]
    );
    assert_eq!(refused, 0);
}

#[test]
fn a_planted_file_on_a_level_one_guest_still_means_no_level_two_tags() {
    let (seen, refused) = gated_session(Some(b"2 feedbeef"), 2, 1);
    assert_eq!(seen.len(), 1);
    assert_eq!(refused, 2);
}
