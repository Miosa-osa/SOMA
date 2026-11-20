//! Reading files, listing directories, and asking what a path is.

use super::{MemoryGuest, code, perform, perform_bytes};
use crate::runner::files::FileCall;

#[test]
fn a_utf8_file_reads_as_text_and_any_other_file_as_base64() {
    let mut guest = MemoryGuest::default();
    guest.seed_file("/workspace/a.txt", "h\u{e9}llo".as_bytes());
    guest.seed_file("/workspace/b.bin", &[0xff, 0xfe, 0x00]);

    let (status, body) = perform_bytes(
        &mut guest,
        FileCall::Read,
        Some("path=/workspace/a.txt"),
        "",
    );
    assert_eq!(status, 200);
    assert_eq!(
        body,
        "{\"content\":\"h\u{e9}llo\",\"path\":\"/workspace/a.txt\"}".as_bytes()
    );

    let (status, body) = perform_bytes(
        &mut guest,
        FileCall::Read,
        Some("path=/workspace/b.bin"),
        "",
    );
    assert_eq!(status, 200);
    assert_eq!(
        body,
        br#"{"content_base64":"//4A","encoding":"base64","path":"/workspace/b.bin"}"#
    );
}

#[test]
fn a_read_names_a_missing_file_and_a_directory_for_what_they_are() {
    let mut guest = MemoryGuest::default();
    guest.seed_directory("/workspace/dir");
    let (status, body) = perform(&mut guest, FileCall::Read, Some("path=/workspace/none"), "");
    assert_eq!((status, code(&body)), (404, "NOT_FOUND"));
    let (status, body) = perform(&mut guest, FileCall::Read, Some("path=/workspace/dir"), "");
    assert_eq!((status, code(&body)), (422, "FS_OPERATION_FAILED"));
}

#[test]
fn a_read_larger_than_the_json_bound_is_sent_to_the_download_route() {
    let mut guest = MemoryGuest::default();
    guest.seed_file(
        "/tmp/big",
        &vec![b'a'; crate::runner::files::MAX_JSON_FILE_BYTES + 1],
    );
    let (status, body) = perform(&mut guest, FileCall::Read, Some("path=/tmp/big"), "");
    assert_eq!((status, code(&body)), (413, "FILE_TOO_LARGE"));
    assert_eq!(body["error"], "file exceeds 32 MiB limit; use fs/download");
}

#[test]
fn a_ranged_read_reports_where_its_bytes_start_and_how_big_the_file_is() {
    let mut guest = MemoryGuest::default();
    guest.seed_file("/tmp/log", b"0123456789");

    let (status, body) = perform_bytes(
        &mut guest,
        FileCall::Read,
        Some("path=/tmp/log&offset=2&length=3"),
        "",
    );
    assert_eq!(status, 200);
    assert_eq!(
        body,
        br#"{"content":"234","length":3,"offset":2,"path":"/tmp/log","size":10}"#
    );

    let (_, body) = perform(
        &mut guest,
        FileCall::Read,
        Some("path=/tmp/log&tail_bytes=4"),
        "",
    );
    assert_eq!(
        (body["content"].as_str(), body["offset"].as_u64()),
        (Some("6789"), Some(6))
    );

    let (_, body) = perform(
        &mut guest,
        FileCall::Read,
        Some("path=/tmp/log&offset=50"),
        "",
    );
    assert_eq!(
        (body["content"].as_str(), body["offset"].as_u64()),
        (Some(""), Some(10))
    );

    let (status, body) = perform(
        &mut guest,
        FileCall::Read,
        Some("path=/tmp/log&offset=-1"),
        "",
    );
    assert_eq!((status, code(&body)), (400, "INVALID_RANGE"));
}

#[test]
fn a_ranged_read_returns_at_most_the_platform_bound() {
    let mut guest = MemoryGuest::default();
    guest.seed_file("/tmp/big", &vec![b'x'; 2 * 1024 * 1024]);
    let (_, body) = perform(
        &mut guest,
        FileCall::Read,
        Some("path=/tmp/big&length=99999999"),
        "",
    );
    assert_eq!(body["length"].as_u64(), Some(524_288));
}

#[test]
fn a_guest_without_ranged_reads_still_reads_a_whole_file() {
    let mut guest = MemoryGuest {
        level_one: true,
        ..MemoryGuest::default()
    };
    guest.seed_file("/tmp/a", b"old guest");
    let (status, body) = perform(&mut guest, FileCall::Read, Some("path=/tmp/a"), "");
    assert_eq!((status, body["content"].as_str()), (200, Some("old guest")));
    assert_eq!(guest.log, ["read_range", "read"]);
}

#[test]
fn a_listing_carries_the_platform_entry_fields() {
    let mut guest = MemoryGuest::default();
    guest.seed_file("/home/user/a.txt", b"abc");
    guest.seed_directory("/home/user/src");

    let (status, body) = perform_bytes(&mut guest, FileCall::List, None, "");

    assert_eq!(status, 200);
    assert_eq!(
        body,
        br#"{"files":[{"modified_at":"2026-10-02T17:04:05Z","name":"a.txt","size_bytes":3,"type":"file"},{"modified_at":"2026-10-02T17:04:05Z","name":"src","size_bytes":0,"type":"directory"}],"path":"/home/user"}"#
    );
}

#[test]
fn a_listing_says_when_the_directory_held_more_than_it_carries() {
    let mut guest = MemoryGuest {
        listing_limit: Some(1),
        ..MemoryGuest::default()
    };
    guest.seed_file("/tmp/a", b"");
    guest.seed_file("/tmp/b", b"");
    let (_, body) = perform(&mut guest, FileCall::List, Some("path=/tmp"), "");
    assert_eq!(body["files"].as_array().map(Vec::len), Some(1));
    assert_eq!(body["truncated"], true);
}

#[test]
fn a_listing_from_a_guest_without_metadata_says_so_instead_of_reporting_zero() {
    let mut guest = MemoryGuest {
        level_one: true,
        ..MemoryGuest::default()
    };
    guest.seed_file("/tmp/a", b"abc");
    let (_, body) = perform_bytes(&mut guest, FileCall::List, Some("path=/tmp"), "");
    assert_eq!(
        body,
        br#"{"files":[{"metadata":"unavailable","name":"a","type":"file"}],"path":"/tmp"}"#
    );
}

#[test]
fn a_listing_leaves_out_a_name_json_cannot_carry_exactly() {
    let entry = soma::FileEntry {
        name: vec![b'a', 0xff],
        kind: soma::FileKind::File,
        metadata: None,
    };
    assert!(crate::runner::files::answer::ListedEntry::of(&entry).is_none());
}

#[test]
fn a_listing_of_a_file_or_of_nothing_is_refused_for_what_it_named() {
    let mut guest = MemoryGuest::default();
    guest.seed_file("/tmp/a", b"");
    let (status, body) = perform(&mut guest, FileCall::List, Some("path=/tmp/a"), "");
    assert_eq!((status, code(&body)), (422, "FS_OPERATION_FAILED"));
    let (status, body) = perform(&mut guest, FileCall::List, Some("path=/tmp/none"), "");
    assert_eq!((status, code(&body)), (404, "NOT_FOUND"));
    let (status, body) = perform(&mut guest, FileCall::List, Some("path="), "");
    assert_eq!((status, code(&body)), (400, "INVALID_PATH"));
}

#[test]
fn a_stat_carries_the_fields_the_platform_stat_prints() {
    let mut guest = MemoryGuest::default();
    guest.seed_file("/workspace/app.js", b"12345");
    let (status, body) = perform_bytes(
        &mut guest,
        FileCall::Stat,
        None,
        r#"{"path":"/workspace/app.js"}"#,
    );
    assert_eq!(status, 200);
    assert_eq!(
        body,
        br#"{"data":{"is_dir":false,"is_file":true,"is_symlink":false,"mode":"0o644","modified_at":"2026-10-02T17:04:05Z","name":"app.js","path":"/workspace/app.js","size":5,"size_bytes":5}}"#
    );
    let (status, body) = perform(
        &mut guest,
        FileCall::Stat,
        None,
        r#"{"path":"/workspace/x"}"#,
    );
    assert_eq!((status, code(&body)), (404, "NOT_FOUND"));
}
