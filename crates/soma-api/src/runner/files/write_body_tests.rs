//! The streaming write-body reader against bodies split at every byte, so no answer depends on
//! where a chunk ended.

use base64::{Engine as _, engine::general_purpose::STANDARD};

use super::{BodyRefused, WriteBody};

/// Feeds `body` in chunks of `size`; the decoded file, the path seen before `content`, and the
/// remaining fields as `finish` returns them.
fn read(
    body: &[u8],
    size: usize,
) -> Result<(Vec<u8>, Option<String>, Option<Vec<u8>>), BodyRefused> {
    let mut reader = WriteBody::new();
    let mut decoded = Vec::new();
    let mut path_at_content = None;
    for chunk in body.chunks(size) {
        reader.feed(chunk, &mut decoded)?;
        if reader.content_started() && path_at_content.is_none() {
            path_at_content = Some(reader.path().map(str::to_owned));
        }
    }
    let path = path_at_content.flatten();
    Ok((decoded, path, reader.finish()?))
}

fn fields(object: &[u8]) -> serde_json::Value {
    serde_json::from_slice(object).expect("the remaining fields are one JSON object")
}

#[test]
fn a_file_decodes_whole_whatever_the_chunk_size() {
    let file: Vec<u8> = (0..=255).cycle().take(4_099).collect();
    let body = format!(
        r#"{{"path":"/tmp/a b\"c","content":"{}","operation_id":null}}"#,
        STANDARD.encode(&file)
    );
    for size in [1, 2, 3, 4, 5, 7, 64, 4_096, body.len()] {
        let (decoded, path, rest) = read(body.as_bytes(), size).expect("well formed");
        assert_eq!(decoded, file, "chunk size {size}");
        assert_eq!(
            path.as_deref(),
            Some("/tmp/a b\"c"),
            "the path arrived before the content"
        );
        assert_eq!(
            fields(&rest.expect("content present")),
            serde_json::json!({"content": "", "path": "/tmp/a b\"c", "operation_id": null})
        );
    }
}

#[test]
fn whitespace_an_escaped_slash_and_nested_values_are_read_as_json_has_them() {
    let body = b" { \"content\" : \"QUJD\\/w==\" ,\n \"path\":\"/x\", \"to\": {\"a\":[1,\"}\"]} , \"n\": 12 } ";
    for size in 1..=body.len() {
        let (decoded, path, rest) = read(body, size).expect("well formed");
        assert_eq!(decoded, STANDARD.decode("QUJD/w==").expect("base64"));
        assert_eq!(path, None, "the path came after the content");
        assert_eq!(
            fields(&rest.expect("content present")),
            serde_json::json!({"content": "", "path": "/x", "to": {"a": [1, "}"]}, "n": 12})
        );
    }
}

#[test]
fn a_body_without_content_says_so_and_an_empty_object_is_one() {
    let (_, _, rest) = read(br#"{"path":"/x"}"#, 3).expect("well formed");
    assert_eq!(rest, None);
    let (_, _, rest) = read(b"{}", 1).expect("well formed");
    assert_eq!(rest, None);
}

#[test]
fn content_that_is_not_base64_is_refused() {
    for body in [
        &br#"{"content":"QUJ"}"#[..],
        br#"{"content":"QU=D"}"#,
        br#"{"content":"QUJD!"}"#,
        br#"{"content":"\nQUJD"}"#,
        br#"{"content":12}"#,
        br#"{"content":"QQ==QUJD"}"#,
    ] {
        assert_eq!(
            read(body, 2).map(|_| ()),
            Err(BodyRefused::Content),
            "{body:?}"
        );
    }
}

#[test]
fn a_body_that_is_not_one_object_or_does_not_end_is_refused() {
    for body in [
        &b"[1]"[..],
        br#"{"path":"/x""#,
        br#"{"path":"/x"} {}"#,
        br#"{"content":"QUJD","content":"QUJD"}"#,
        b"{path:1}",
    ] {
        assert_eq!(
            read(body, 3).map(|_| ()),
            Err(BodyRefused::Malformed),
            "{body:?}"
        );
    }
}

#[test]
fn oversized_fields_are_refused_rather_than_held() {
    let body = format!(r#"{{"path":"/{}","content":""}}"#, "a".repeat(70 * 1024));
    assert_eq!(
        read(body.as_bytes(), 4_096).map(|_| ()),
        Err(BodyRefused::Malformed)
    );
}
