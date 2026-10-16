//! The host gates an API level 2 request behind the level the guest's own probe answered.

use std::thread;

use crate::{FileFailure, FileOutcome, FileRequest, GuestMessage, HostMessage};

use super::host_file::repaired_host;

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
