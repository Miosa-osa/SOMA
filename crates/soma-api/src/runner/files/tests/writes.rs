//! Writing a file: the atomic replacement, appending, modes, and the refusals of a write body.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::json;

use super::{MemoryGuest, code, leftover_temporaries, perform, perform_bytes};
use crate::runner::files::FileCall;

fn write(guest: &mut MemoryGuest, body: &serde_json::Value) -> (u16, serde_json::Value) {
    perform(guest, FileCall::Write, None, &body.to_string())
}

#[test]
fn a_write_replaces_the_file_through_a_temporary_sibling() {
    let mut guest = MemoryGuest::default();
    guest.seed_file("/workspace/a.txt", b"old contents");

    let (status, body) = perform_bytes(
        &mut guest,
        FileCall::Write,
        None,
        r#"{"path":"/workspace/a.txt","content":"new"}"#,
    );

    assert_eq!(status, 201);
    assert_eq!(
        body,
        br#"{"file":{"filename":"a.txt","path":"/workspace/a.txt","size_bytes":3},"success":true}"#
    );
    assert_eq!(guest.file("/workspace/a.txt"), Some(&b"new"[..]));
    assert_eq!(guest.mode("/workspace/a.txt"), Some(0o644));
    assert!(leftover_temporaries(&guest).is_empty());
    assert_eq!(guest.log, ["mkdir", "write_range", "chmod", "rename"]);
}

#[test]
fn a_write_makes_the_missing_parent_directories_and_echoes_a_relative_path() {
    let mut guest = MemoryGuest::default();
    let (status, body) = write(
        &mut guest,
        &json!({"path": "src/deep/main.js", "content": "1"}),
    );
    assert_eq!(status, 201);
    assert_eq!(body["file"]["path"], "src/deep/main.js");
    assert_eq!(guest.file("/home/user/src/deep/main.js"), Some(&b"1"[..]));
}

#[test]
fn a_file_larger_than_one_transfer_is_written_in_ranges_and_reads_back_identical() {
    let contents: Vec<u8> = (0..9 * 1024 * 1024 + 7_usize)
        .map(|index| u8::try_from(index * 31 % 251).expect("below 251"))
        .collect();
    let mut guest = MemoryGuest::default();
    let (status, _) = write(
        &mut guest,
        &json!({"path": "/tmp/large.bin", "content_base64": STANDARD.encode(&contents)}),
    );
    assert_eq!(status, 201);
    assert_eq!(
        guest
            .log
            .iter()
            .filter(|name| **name == "write_range")
            .count(),
        3,
        "nine mebibytes and a little are three transfers"
    );
    guest.log.clear();

    let (status, body) = perform(&mut guest, FileCall::Read, Some("path=/tmp/large.bin"), "");

    assert_eq!(status, 200);
    assert_eq!(body["encoding"], "base64");
    let read = STANDARD
        .decode(body["content_base64"].as_str().expect("base64 contents"))
        .expect("decodes");
    assert_eq!(read, contents);
    assert_eq!(
        guest
            .log
            .iter()
            .filter(|name| **name == "read_range")
            .count(),
        3
    );
}

#[test]
fn an_empty_write_creates_an_empty_file() {
    let mut guest = MemoryGuest::default();
    let (status, body) = write(&mut guest, &json!({"path": "/tmp/empty", "content": ""}));
    assert_eq!(
        (status, body["file"]["size_bytes"].as_u64()),
        (201, Some(0))
    );
    assert_eq!(guest.file("/tmp/empty"), Some(&b""[..]));
}

#[test]
fn base64_content_may_also_arrive_as_content_with_an_encoding() {
    let mut guest = MemoryGuest::default();
    let encoded = STANDARD.encode([0xff_u8, 0x00, 0x80]);
    let (status, _) = write(
        &mut guest,
        &json!({"path": "/tmp/b", "content": encoded, "encoding": "base64"}),
    );
    assert_eq!(status, 201);
    assert_eq!(guest.file("/tmp/b"), Some(&[0xff_u8, 0x00, 0x80][..]));
}

#[test]
fn a_mode_applies_only_to_a_file_the_write_creates() {
    let mut guest = MemoryGuest::default();
    write(
        &mut guest,
        &json!({"path": "/tmp/new", "content": "x", "mode": "0600"}),
    );
    assert_eq!(guest.mode("/tmp/new"), Some(0o600));

    guest.seed_file("/tmp/old", b"x");
    write(
        &mut guest,
        &json!({"path": "/tmp/old", "content": "y", "mode": "0600"}),
    );
    assert_eq!(
        guest.mode("/tmp/old"),
        Some(0o644),
        "as the platform's daemon replaces it"
    );
}

#[test]
fn an_append_adds_to_an_existing_file_and_creates_a_missing_one() {
    let mut guest = MemoryGuest::default();
    guest.seed_file("/var/log/app.log", b"one\n");
    let (status, body) = write(
        &mut guest,
        &json!({"path": "/var/log/app.log", "content": "two\n", "append": true}),
    );
    assert_eq!(
        (status, body["file"]["size_bytes"].as_u64()),
        (201, Some(8))
    );
    assert_eq!(guest.file("/var/log/app.log"), Some(&b"one\ntwo\n"[..]));

    write(
        &mut guest,
        &json!({"path": "/var/log/fresh.log", "content": "a", "append": true}),
    );
    assert_eq!(guest.file("/var/log/fresh.log"), Some(&b"a"[..]));
}

#[test]
fn a_write_that_fails_to_publish_leaves_neither_its_sibling_nor_a_changed_target() {
    let mut guest = MemoryGuest::default();
    guest.seed_file("/workspace/keep.txt", b"kept");
    guest.refuse_renames = Some(soma::FileRefusal::Denied);

    let (status, body) = write(
        &mut guest,
        &json!({"path": "/workspace/keep.txt", "content": "lost"}),
    );

    assert_eq!((status, code(&body)), (403, "PERMISSION_DENIED"));
    assert_eq!(guest.file("/workspace/keep.txt"), Some(&b"kept"[..]));
    assert!(
        leftover_temporaries(&guest).is_empty(),
        "{:?}",
        leftover_temporaries(&guest)
    );
}

#[test]
fn a_write_onto_a_directory_names_the_wrong_kind() {
    let mut guest = MemoryGuest::default();
    guest.seed_directory("/workspace/dir");
    let (status, body) = write(
        &mut guest,
        &json!({"path": "/workspace/dir", "content": "x"}),
    );
    assert_eq!((status, code(&body)), (422, "FS_OPERATION_FAILED"));
    assert!(leftover_temporaries(&guest).is_empty());
}

#[test]
fn a_malformed_write_body_is_refused_with_the_platform_message() {
    let cases = [
        (
            json!({"path": "/tmp/a"}),
            "content or content_base64 is required",
        ),
        (
            json!({"path": "/tmp/a", "content_base64": "@@"}),
            "content_base64 is not valid base64",
        ),
        (
            json!({"path": "/tmp/a", "content": "x", "encoding": "hex"}),
            "encoding must be utf8 or base64",
        ),
        (
            json!({"path": "/tmp/a", "content": "x", "append": "yes"}),
            "append must be true or false",
        ),
        (
            json!({"path": "/tmp/a", "content": "x", "mode": "rw"}),
            "mode must be octal such as 0644",
        ),
        (
            json!({"path": "/tmp/a", "content": "x", "mode": "4755"}),
            "the set-user, set-group and sticky bits cannot be set through the file API",
        ),
    ];
    for (body, message) in cases {
        let mut guest = MemoryGuest::default();
        let (status, answer) = write(&mut guest, &body);
        assert_eq!((status, code(&answer)), (400, "INVALID_BODY"), "{body}");
        assert_eq!(answer["error"], message);
        assert!(guest.log.is_empty());
    }
}

#[test]
fn a_write_larger_than_the_json_bound_is_sent_to_the_upload_route() {
    let mut guest = MemoryGuest::default();
    let content = "a".repeat(crate::runner::files::MAX_JSON_FILE_BYTES + 1);
    let (status, body) = write(&mut guest, &json!({"path": "/tmp/big", "content": content}));
    assert_eq!((status, code(&body)), (413, "FILE_TOO_LARGE"));
    assert_eq!(body["error"], "file exceeds 32 MiB limit; use fs/upload");
    assert!(guest.log.is_empty());
}
