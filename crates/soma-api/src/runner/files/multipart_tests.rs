//! The multipart reader against bodies split at every byte, so no event depends on where a
//! chunk happened to end.

use super::{Event, Malformed, Multipart, find};

const TYPE: &str = "multipart/form-data; boundary=XyZ";

/// One part as a test writes it: form name, filename, and contents.
type Written<'a> = (&'a str, Option<&'a str>, &'a [u8]);
/// One part as the reader returned it.
type Read = (String, Option<String>, Vec<u8>);

fn body(parts: &[Written<'_>]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, filename, contents) in parts {
        body.extend_from_slice(b"--XyZ\r\n");
        let disposition = match filename {
            Some(filename) => format!("form-data; name=\"{name}\"; filename=\"{filename}\""),
            None => format!("form-data; name=\"{name}\""),
        };
        body.extend_from_slice(format!("Content-Disposition: {disposition}\r\n").as_bytes());
        body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
        body.extend_from_slice(contents);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"--XyZ--\r\n");
    body
}

/// Feeds `body` in chunks of `size` and folds the events into whole parts.
fn parts(body: &[u8], size: usize) -> Result<Vec<Read>, Malformed> {
    let mut reader = Multipart::for_content_type(TYPE).expect("a boundary");
    let mut events = Vec::new();
    for chunk in body.chunks(size) {
        reader.feed(chunk, &mut events)?;
    }
    assert!(reader.finished(), "the closing delimiter was read");
    let mut parts = Vec::new();
    for event in events {
        match event {
            Event::Begin { name, filename } => parts.push((name, filename, Vec::new())),
            Event::Data(bytes) => parts.last_mut().expect("a part").2.extend(bytes),
            Event::End => {}
        }
    }
    Ok(parts)
}

#[test]
fn fields_and_a_file_come_back_whatever_the_chunk_size() {
    // Near misses of the delimiter: another boundary, the boundary without its line end, and a
    // line end followed by the boundary's first bytes only.
    let file: Vec<u8> = b"line one\r\n--XyA almost\r\nend--XyZ no line end\r\n\r\n--Xy"
        .iter()
        .copied()
        .chain((0..=255).collect::<Vec<u8>>())
        .collect();
    let body = body(&[
        ("path", None, b"/workspace/"),
        ("file", Some("a.bin"), &file),
    ]);
    for size in 1..=body.len() {
        let parts = parts(&body, size).expect("well formed");
        assert_eq!(parts.len(), 2, "chunk size {size}");
        assert_eq!(parts[0], ("path".to_owned(), None, b"/workspace/".to_vec()));
        assert_eq!(
            parts[1],
            ("file".to_owned(), Some("a.bin".to_owned()), file.clone()),
            "chunk size {size}"
        );
    }
}

#[test]
fn a_preamble_an_empty_part_and_an_epilogue_are_read_as_rfc_2046_has_them() {
    let mut text = b"ignored preamble\r\n".to_vec();
    text.extend(body(&[("empty", Some(""), b"")]));
    text.extend_from_slice(b"ignored epilogue");
    let parts = parts(&text, 3).expect("well formed");
    assert_eq!(
        parts,
        [("empty".to_owned(), Some(String::new()), Vec::new())]
    );
}

#[test]
fn a_quoted_filename_keeps_its_escaped_characters_and_semicolons() {
    let mut text = b"--XyZ\r\ncontent-disposition: form-data; name=\"file\"; filename=\"a\\\"b;c.txt\"\r\n\r\nx\r\n--XyZ--".to_vec();
    text.extend_from_slice(b"\r\n");
    let parts = parts(&text, 7).expect("well formed");
    assert_eq!(parts[0].1.as_deref(), Some("a\"b;c.txt"));
}

#[test]
fn a_part_without_a_form_name_or_with_endless_headers_is_refused() {
    let mut reader = Multipart::for_content_type(TYPE).expect("a boundary");
    let mut events = Vec::new();
    assert_eq!(
        reader.feed(b"--XyZ\r\nContent-Type: text/plain\r\n\r\nx", &mut events),
        Err(Malformed)
    );

    let mut reader = Multipart::for_content_type(TYPE).expect("a boundary");
    let mut endless = b"--XyZ\r\nX-Pad: ".to_vec();
    endless.extend(std::iter::repeat_n(b'a', 20 * 1024));
    assert_eq!(reader.feed(&endless, &mut events), Err(Malformed));

    let mut reader = Multipart::for_content_type(TYPE).expect("a boundary");
    assert_eq!(reader.feed(b"--XyZjunk", &mut events), Err(Malformed));
}

#[test]
fn a_body_that_stops_before_its_closing_delimiter_is_not_finished() {
    let body = body(&[("file", Some("a"), b"abc")]);
    let mut reader = Multipart::for_content_type(TYPE).expect("a boundary");
    let mut events = Vec::new();
    reader
        .feed(&body[..body.len() - 9], &mut events)
        .expect("well formed so far");
    assert!(!reader.finished());
}

#[test]
fn only_a_form_data_type_with_a_usable_boundary_is_read() {
    assert!(Multipart::for_content_type("multipart/form-data; boundary=\"q b\"").is_some());
    assert!(Multipart::for_content_type("Multipart/Form-Data;charset=utf-8; BOUNDARY=x").is_some());
    for refused in [
        "application/json",
        "multipart/form-data",
        "multipart/mixed; boundary=x",
        "multipart/form-data; boundary=",
    ] {
        assert!(Multipart::for_content_type(refused).is_none(), "{refused}");
    }
    let long = format!("multipart/form-data; boundary={}", "b".repeat(71));
    assert!(Multipart::for_content_type(&long).is_none());
}

#[test]
fn the_search_finds_a_needle_at_every_position_and_nowhere_else() {
    let needle = b"\r\n--b";
    for at in 0..20 {
        let mut haystack = vec![b'\r'; at];
        haystack.extend_from_slice(needle);
        assert_eq!(find(&haystack, needle), Some(at));
    }
    assert_eq!(find(b"\r\n--", needle), None);
    assert_eq!(find(b"", needle), None);
}
