//! The six filesystem routes, over the same bytes a client would send.
//!
//! These prove the shape of the service: that each route reaches the engine, that the answer
//! document says what happened, and that bytes survive the round trip. What the guest itself does
//! with a path is proved live, against a real sandbox, and recorded under `docs/evidence`.

#[allow(
    dead_code,
    reason = "each integration test uses a subset of the shared fixtures"
)]
mod support;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use support::{Call, FIXTURE_INSTANCE_ID, FakeFacade, Mode, call, identified};

/// Bytes that are not valid UTF-8, and that a text-only transfer would silently mangle.
const BINARY: &[u8] = &[0x00, 0xff, 0xfe, 0x80, 0x0a, 0x7f, 0xc3, 0x28, 0x00];

fn post(facade: &mut FakeFacade, operation: &str, body: &str) -> (u16, serde_json::Value) {
    let request = identified(
        "POST",
        &format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}/filesystem/{operation}"),
        body,
    );
    call(facade, &request)
}

#[test]
fn a_file_written_reads_back_as_the_same_bytes() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let encoded = STANDARD.encode(BINARY);

    let (status, written) = post(
        &mut facade,
        "write",
        &format!(r#"{{"path":"/workspace/blob.bin","content":"{encoded}"}}"#),
    );
    assert_eq!(status, 200);
    assert_eq!(written["result"]["byte_length"], BINARY.len());

    let (status, read) = post(&mut facade, "read", r#"{"path":"/workspace/blob.bin"}"#);
    assert_eq!(status, 200);
    assert_eq!(read["result"]["content"]["encoding"], "base64");
    assert_eq!(read["result"]["content"]["byte_length"], BINARY.len());
    let returned = STANDARD
        .decode(
            read["result"]["content"]["data"]
                .as_str()
                .expect("the content is a base64 string"),
        )
        .expect("the service encodes valid base64");
    assert_eq!(returned, BINARY, "the bytes must survive unchanged");
}

#[test]
fn a_written_file_is_listed_exists_and_is_gone_once_removed() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let encoded = STANDARD.encode(b"hello");
    post(
        &mut facade,
        "write",
        &format!(r#"{{"path":"/workspace/hello.txt","content":"{encoded}"}}"#),
    );

    let (_, listed) = post(&mut facade, "list", r#"{"path":"/workspace"}"#);
    let names: Vec<String> = listed["result"]["entries"]
        .as_array()
        .expect("a listing carries entries")
        .iter()
        .map(|entry| {
            String::from_utf8(
                STANDARD
                    .decode(entry["name"]["data"].as_str().expect("a base64 name"))
                    .expect("valid base64"),
            )
            .expect("this fixture's names are text")
        })
        .collect();
    assert_eq!(names, ["hello.txt"]);
    assert_eq!(listed["result"]["more_entries"], false);

    let (_, present) = post(&mut facade, "exists", r#"{"path":"/workspace/hello.txt"}"#);
    assert_eq!(present["result"]["exists"], true);
    assert_eq!(present["result"]["kind"], "file");

    let (status, removed) = post(&mut facade, "remove", r#"{"path":"/workspace/hello.txt"}"#);
    assert_eq!(status, 200);
    assert!(removed["result"]["refusal"].is_null());

    let (_, absent) = post(&mut facade, "exists", r#"{"path":"/workspace/hello.txt"}"#);
    assert_eq!(absent["result"]["exists"], false);
    assert!(absent["result"]["kind"].is_null());
}

#[test]
fn making_a_directory_reaches_the_engine_and_reports_nothing_else() {
    let mut facade = FakeFacade::new(Mode::Succeed);

    let (status, body) = post(&mut facade, "mkdir", r#"{"path":"/workspace/nested"}"#);

    assert_eq!(status, 200);
    assert_eq!(facade.calls, [Call::File]);
    assert_eq!(body["result"]["operation"], "mkdir");
    assert!(body["result"]["content"].is_null());
    assert!(body["result"]["entries"].is_null());
}

/// A cause the guest reported is carried as a typed refusal, not as a host error.
#[test]
fn a_guest_refusal_is_a_typed_cause_rather_than_a_backend_fault() {
    let mut facade = FakeFacade::new(Mode::Succeed);

    let (status, absent) = post(&mut facade, "read", r#"{"path":"/workspace/missing"}"#);
    assert_eq!(status, 200, "the operation happened; the guest declined it");
    assert_eq!(absent["result"]["refusal"], "not_found");

    let encoded = STANDARD.encode(b"x");
    let (status, denied) = post(
        &mut facade,
        "write",
        &format!(r#"{{"path":"/readonly/main.js","content":"{encoded}"}}"#),
    );
    assert_eq!(status, 200);
    assert_eq!(denied["result"]["refusal"], "denied");
}

#[test]
fn a_field_the_operation_does_not_use_is_refused_rather_than_ignored() {
    let mut facade = FakeFacade::new(Mode::Succeed);

    let (status, body) = post(
        &mut facade,
        "list",
        r#"{"path":"/workspace","recursive":true}"#,
    );

    assert_eq!(status, 400);
    assert!(facade.calls.is_empty(), "it must not reach the engine");
    assert_eq!(body["error"]["code"], "invalid_input");
}

#[test]
fn a_write_without_content_never_reaches_the_engine() {
    let mut facade = FakeFacade::new(Mode::Succeed);

    let (status, _) = post(&mut facade, "write", r#"{"path":"/workspace/main.js"}"#);

    assert_eq!(status, 400);
    assert!(facade.calls.is_empty());
}

#[test]
fn content_beyond_the_transfer_bound_is_refused_before_the_engine() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let oversized = STANDARD.encode(vec![0_u8; soma::MAX_FILE_BYTES + 1]);

    let (status, body) = post(
        &mut facade,
        "write",
        &format!(r#"{{"path":"/workspace/big.bin","content":"{oversized}"}}"#),
    );

    assert_eq!(status, 413);
    assert!(facade.calls.is_empty());
    assert_eq!(body["error"]["code"], "content_too_large");
}
