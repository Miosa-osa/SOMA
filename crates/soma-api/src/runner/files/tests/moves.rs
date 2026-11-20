//! Making, moving, removing, and changing the mode of paths.

use super::{MemoryGuest, code, perform, perform_bytes};
use crate::runner::files::FileCall;

#[test]
fn a_directory_is_made_with_its_parents() {
    let mut guest = MemoryGuest::default();
    let (status, body) = perform_bytes(
        &mut guest,
        FileCall::MakeDirectory,
        None,
        r#"{"path":"/workspace/a/b/c","recursive":true}"#,
    );
    assert_eq!(status, 201);
    assert_eq!(body, br#"{"data":{"status":"ok"}}"#);
    assert_eq!(guest.mode("/workspace/a/b/c"), Some(0o755));

    let (status, _) = perform(
        &mut guest,
        FileCall::MakeDirectory,
        None,
        r#"{"path":"/workspace/a"}"#,
    );
    assert_eq!(
        status, 201,
        "a directory already there is the directory asked for"
    );

    guest.seed_file("/workspace/file", b"");
    let (status, body) = perform(
        &mut guest,
        FileCall::MakeDirectory,
        None,
        r#"{"path":"/workspace/file"}"#,
    );
    assert_eq!((status, code(&body)), (409, "ALREADY_EXISTS"));
}

#[test]
fn a_rename_replaces_its_destination_and_echoes_both_normalized_paths() {
    let mut guest = MemoryGuest::default();
    guest.seed_file("/home/user/a.txt", b"a");
    guest.seed_file("/home/user/b.txt", b"b");

    let (status, body) = perform_bytes(
        &mut guest,
        FileCall::Rename,
        None,
        r#"{"from":"a.txt","to":"./b.txt"}"#,
    );

    assert_eq!(status, 200);
    assert_eq!(
        body,
        br#"{"data":{"from":"/home/user/a.txt","to":"/home/user/b.txt"}}"#
    );
    assert_eq!(guest.file("/home/user/b.txt"), Some(&b"a"[..]));
    assert_eq!(guest.file("/home/user/a.txt"), None);
}

#[test]
fn a_rename_into_an_existing_directory_moves_the_source_inside_it_as_mv_does() {
    let mut guest = MemoryGuest::default();
    guest.seed_file("/tmp/report.csv", b"1,2");
    guest.seed_directory("/tmp/archive");
    let (status, _) = perform(
        &mut guest,
        FileCall::Rename,
        None,
        r#"{"from":"/tmp/report.csv","to":"/tmp/archive"}"#,
    );
    assert_eq!(status, 200);
    assert_eq!(guest.file("/tmp/archive/report.csv"), Some(&b"1,2"[..]));
}

#[test]
fn a_rename_makes_the_destination_directory_and_moves_a_whole_tree() {
    let mut guest = MemoryGuest::default();
    guest.seed_file("/workspace/src/lib/a.rs", b"a");
    let (status, _) = perform(
        &mut guest,
        FileCall::Rename,
        None,
        r#"{"from":"/workspace/src","to":"/workspace/new/place/src"}"#,
    );
    assert_eq!(status, 200);
    assert_eq!(
        guest.file("/workspace/new/place/src/lib/a.rs"),
        Some(&b"a"[..])
    );
    assert_eq!(guest.file("/workspace/src/lib/a.rs"), None);
}

#[test]
fn a_rename_names_what_is_missing() {
    let mut guest = MemoryGuest::default();
    let (status, body) = perform(&mut guest, FileCall::Rename, None, r#"{"from":"/tmp/a"}"#);
    assert_eq!((status, code(&body)), (400, "MISSING_PARAMS"));
    let (status, body) = perform(
        &mut guest,
        FileCall::Rename,
        None,
        r#"{"from":"/tmp/none","to":"/tmp/b"}"#,
    );
    assert_eq!((status, code(&body)), (404, "NOT_FOUND"));
}

#[test]
fn a_removal_of_what_is_absent_succeeds_as_rm_f_does() {
    let mut guest = MemoryGuest::default();
    guest.seed_file("/tmp/a", b"");
    let (status, body) = perform_bytes(&mut guest, FileCall::Delete, Some("path=/tmp/a"), "");
    assert_eq!(status, 200);
    assert_eq!(body, br#"{"path":"/tmp/a","success":true}"#);
    assert_eq!(guest.file("/tmp/a"), None);
    let (status, _) = perform(&mut guest, FileCall::Delete, Some("path=/tmp/a"), "");
    assert_eq!(status, 200);
}

#[test]
fn a_full_directory_is_removed_only_when_the_removal_is_recursive() {
    let mut guest = MemoryGuest::default();
    guest.seed_file("/workspace/dir/a", b"");
    let (status, body) = perform(
        &mut guest,
        FileCall::Delete,
        Some("path=/workspace/dir"),
        "",
    );
    assert_eq!((status, code(&body)), (422, "FS_OPERATION_FAILED"));
    assert_eq!(body["error"], "delete failed: the directory is not empty");

    let (status, _) = perform(
        &mut guest,
        FileCall::Delete,
        Some("path=/workspace/dir&recursive=true"),
        "",
    );
    assert_eq!(status, 200);
    assert!(!guest.nodes.contains_key("/workspace/dir"));

    guest.seed_file("/workspace/again/a", b"");
    let (status, _) = perform(
        &mut guest,
        FileCall::Delete,
        None,
        r#"{"path":"/workspace/again","recursive":true}"#,
    );
    assert_eq!(
        status, 200,
        "a JSON body may carry the flag, as on the platform"
    );
}

#[test]
fn a_mode_is_set_from_octal_and_from_a_symbolic_clause() {
    let mut guest = MemoryGuest::default();
    guest.seed_file("/workspace/run.sh", b"#!/bin/sh");

    let (status, body) = perform_bytes(
        &mut guest,
        FileCall::Chmod,
        None,
        r#"{"path":"/workspace/run.sh","mode":"0700"}"#,
    );
    assert_eq!(status, 200);
    assert_eq!(
        body,
        br#"{"data":{"mode":"0700","path":"/workspace/run.sh"}}"#
    );
    assert_eq!(guest.mode("/workspace/run.sh"), Some(0o700));

    let (status, _) = perform(
        &mut guest,
        FileCall::Chmod,
        None,
        r#"{"path":"/workspace/run.sh","mode":"go+rx"}"#,
    );
    assert_eq!(status, 200);
    assert_eq!(guest.mode("/workspace/run.sh"), Some(0o755));

    let (_, body) = perform(
        &mut guest,
        FileCall::Chmod,
        None,
        r#"{"path":"/workspace/run.sh","mode":420}"#,
    );
    assert_eq!(body["data"]["mode"], "0644");
}

#[test]
fn a_mode_change_names_what_is_wrong_with_it() {
    let mut guest = MemoryGuest::default();
    guest.seed_file("/tmp/a", b"");
    for (body, status, expected) in [
        (r#"{"path":"/tmp/a"}"#, 400, "MISSING_MODE"),
        (r#"{"path":"/tmp/a","mode":"9"}"#, 400, "INVALID_MODE"),
        (r#"{"path":"/tmp/a","mode":"u+s"}"#, 400, "INVALID_MODE"),
        (r#"{"path":"/tmp/none","mode":"0644"}"#, 404, "NOT_FOUND"),
    ] {
        let (got, answer) = perform(&mut guest, FileCall::Chmod, None, body);
        assert_eq!((got, code(&answer)), (status, expected), "{body}");
    }
}
