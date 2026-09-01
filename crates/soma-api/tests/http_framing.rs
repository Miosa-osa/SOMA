use std::io::{self, Cursor, Write};

use soma_api::{Method, Request, Response};

fn parse(raw: &str) -> Result<Request, u16> {
    let mut reader = Cursor::new(raw.as_bytes().to_vec());
    Request::read_from(&mut reader).map_err(soma_api::ApiError::status)
}

#[test]
fn a_request_line_body_and_headers_are_parsed_into_their_parts() {
    let request =
        parse("POST /v1/sandboxes HTTP/1.1\r\nx-soma-tenant: acme\r\ncontent-length: 2\r\n\r\n{}")
            .expect("the request parses");

    assert_eq!(request.method, Method::Post);
    assert_eq!(request.path, "/v1/sandboxes");
    assert_eq!(request.header("X-Soma-Tenant"), Some("acme"));
    assert_eq!(request.body, b"{}");
}

#[test]
fn a_query_string_is_stripped_from_the_routed_path() {
    let request = parse("GET /v1/sandboxes?state=ready HTTP/1.1\r\n\r\n").expect("parses");

    assert_eq!(request.path, "/v1/sandboxes");
    assert_eq!(request.segments(), vec!["v1", "sandboxes"]);
}

#[test]
fn a_body_without_a_content_length_is_refused_rather_than_guessed() {
    assert_eq!(
        parse("POST /v1/sandboxes HTTP/1.1\r\ntransfer-encoding: chunked\r\n\r\n0\r\n\r\n").err(),
        Some(411)
    );
}

/// The refused length is derived from the declared allowance rather than written out.
///
/// The allowance is itself derived from the largest file write this service admits, so a literal
/// here would silently stop testing the boundary the first time that bound moved.
#[test]
fn a_content_length_beyond_the_declared_allowance_is_refused() {
    let beyond = soma_api::http::request::MAX_BODY_BYTES + 1;
    assert_eq!(
        parse(&format!(
            "POST /v1/sandboxes HTTP/1.1\r\ncontent-length: {beyond}\r\n\r\n"
        ))
        .err(),
        Some(413)
    );
}

#[test]
fn a_body_shorter_than_its_content_length_is_refused() {
    assert_eq!(
        parse("POST /v1/sandboxes HTTP/1.1\r\ncontent-length: 64\r\n\r\n{}").err(),
        Some(400)
    );
}

#[test]
fn an_unknown_protocol_version_is_refused() {
    assert_eq!(parse("GET /v1/sandboxes HTTP/2.0\r\n\r\n").err(), Some(400));
}

#[test]
fn http11_is_persistent_unless_the_client_closes_it() {
    let persistent = parse("GET /v1/sandboxes HTTP/1.1\r\n\r\n").expect("parses");
    let closing = parse("GET /v1/sandboxes HTTP/1.1\r\nconnection: close\r\n\r\n").expect("parses");

    assert!(persistent.keep_alive());
    assert!(!closing.keep_alive());
}

#[test]
fn a_request_target_that_is_not_a_path_is_refused() {
    assert_eq!(
        parse("GET http://elsewhere/v1/sandboxes HTTP/1.1\r\n\r\n").err(),
        Some(400)
    );
}

#[test]
fn an_oversized_request_line_is_refused_before_it_is_kept() {
    let raw = format!("GET /{} HTTP/1.1\r\n\r\n", "a".repeat(16 * 1024));

    assert_eq!(parse(&raw).err(), Some(431));
}

#[test]
fn more_headers_than_the_service_accepts_are_refused() {
    let headers = (0..128).fold(String::new(), |mut headers, index| {
        use std::fmt::Write as _;
        let _ = write!(headers, "x-filler-{index}: value\r\n");
        headers
    });

    assert_eq!(
        parse(&format!("GET /v1/sandboxes HTTP/1.1\r\n{headers}\r\n")).err(),
        Some(431)
    );
}

#[test]
fn a_response_is_written_as_a_closing_json_exchange() {
    let mut written = Vec::new();

    Response::new(201, b"{\"status\":\"ok\"}".to_vec())
        .write_to(&mut written)
        .expect("writing to a vector cannot fail");

    let text = String::from_utf8(written).expect("the response is UTF-8");
    assert!(text.starts_with("HTTP/1.1 201 Created\r\n"));
    assert!(text.contains("content-type: application/json\r\n"));
    assert!(text.contains("content-length: 15\r\n"));
    assert!(text.contains("connection: close\r\n"));
    assert!(text.ends_with("\r\n\r\n{\"status\":\"ok\"}"));
}

#[test]
fn a_response_is_emitted_in_one_write() {
    #[derive(Default)]
    struct CountingWriter {
        bytes: Vec<u8>,
        writes: usize,
    }

    impl Write for CountingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let mut writer = CountingWriter::default();
    Response::new(200, b"{}".to_vec())
        .write_to(&mut writer)
        .expect("writing succeeds");

    assert_eq!(writer.writes, 1);
    assert!(writer.bytes.ends_with(b"\r\n\r\n{}"));
}

#[test]
fn a_persistent_response_declares_keep_alive() {
    let mut written = Vec::new();

    Response::new(200, b"{}".to_vec())
        .write_keep_alive_to(&mut written)
        .expect("writing succeeds");

    assert!(
        written
            .windows(24)
            .any(|part| part == b"connection: keep-alive\r\n")
    );
}
