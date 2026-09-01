use std::io::BufRead;

use crate::envelope::ApiError;

/// Largest accepted request line, generous for a path of bounded identifiers.
pub const MAX_REQUEST_LINE_BYTES: usize = 8 * 1024;
/// Largest accepted single header line.
pub const MAX_HEADER_LINE_BYTES: usize = 8 * 1024;
/// Largest accepted header count.
pub const MAX_HEADERS: usize = 64;
/// Largest accepted body.
///
/// It is derived rather than chosen, because the largest body this service must accept is a
/// maximal file write, and a limit picked independently of that would refuse a write the rest of
/// the stack is willing to perform. Base64 turns every three bytes into four, and the remaining
/// allowance covers the path, the identities, and the JSON framing around them.
pub const MAX_BODY_BYTES: usize = base64_length(soma::MAX_FILE_BYTES) + FRAMING_ALLOWANCE;

/// What the rest of one filesystem body can take: a maximal path and the JSON around it.
const FRAMING_ALLOWANCE: usize = 64 * 1024;

/// The encoded length of `bytes` bytes of base64.
const fn base64_length(bytes: usize) -> usize {
    bytes.div_ceil(3) * 4
}

// A maximal direct command must still fit, which it did under the fixed limit this replaced.
const _: () = assert!(MAX_BODY_BYTES >= 2 * 1024 * 1024);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Method {
    Get,
    Post,
    Delete,
    Other,
}

/// One parsed HTTP request, reduced to what the router needs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub method: Method,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    http11: bool,
}

impl Request {
    /// Looks up a header by name, case insensitively as HTTP requires.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Splits the path into non-empty segments.
    #[must_use]
    pub fn segments(&self) -> Vec<&str> {
        self.path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect()
    }

    /// Whether another request may follow this one on the same connection.
    #[must_use]
    pub fn keep_alive(&self) -> bool {
        let connection = self.header("connection");
        if self.http11 {
            !connection.is_some_and(|value| value.eq_ignore_ascii_case("close"))
        } else {
            connection.is_some_and(|value| value.eq_ignore_ascii_case("keep-alive"))
        }
    }

    /// Reads one request from a buffered stream.
    ///
    /// Every bound is enforced while reading rather than after, so a hostile or broken client
    /// cannot make the service allocate on its behalf before being refused.
    ///
    /// # Errors
    ///
    /// Returns a 400 refusal for a malformed request, a 411 refusal when a body is sent without
    /// a length, and a 413 refusal when any declared bound is exceeded.
    pub fn read_from(reader: &mut impl BufRead) -> Result<Self, ApiError> {
        let line = read_line(reader, MAX_REQUEST_LINE_BYTES)?;
        let (method, path, http11) = request_line(&line)?;
        let headers = read_headers(reader)?;
        let body = read_body(reader, &headers)?;
        Ok(Self {
            method,
            path,
            headers,
            body,
            http11,
        })
    }
}

fn request_line(line: &str) -> Result<(Method, String, bool), ApiError> {
    let mut parts = line.split(' ');
    let (Some(method), Some(target), Some(version)) = (parts.next(), parts.next(), parts.next())
    else {
        return Err(ApiError::invalid("the request line is malformed"));
    };
    if parts.next().is_some() || !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        return Err(ApiError::invalid("the request line is malformed"));
    }
    // The query string is split off and discarded: no route on this service reads one, and
    // silently ignoring a trailing query would let a caller believe a filter had been applied.
    let path = target.split(['?', '#']).next().unwrap_or_default();
    if path.is_empty() || !path.starts_with('/') {
        return Err(ApiError::invalid("the request target is malformed"));
    }
    Ok((
        match method {
            "GET" => Method::Get,
            "POST" => Method::Post,
            "DELETE" => Method::Delete,
            _ => Method::Other,
        },
        path.to_owned(),
        version == "HTTP/1.1",
    ))
}

fn read_headers(reader: &mut impl BufRead) -> Result<Vec<(String, String)>, ApiError> {
    let mut headers = Vec::new();
    loop {
        let line = read_line(reader, MAX_HEADER_LINE_BYTES)?;
        if line.is_empty() {
            return Ok(headers);
        }
        if headers.len() == MAX_HEADERS {
            return Err(ApiError::new(
                431,
                "header_limit",
                "the request carried more headers than the service accepts",
                false,
            ));
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(ApiError::invalid("a request header is malformed"));
        };
        headers.push((name.trim().to_owned(), value.trim().to_owned()));
    }
}

fn read_body(reader: &mut impl BufRead, headers: &[(String, String)]) -> Result<Vec<u8>, ApiError> {
    if headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("transfer-encoding"))
    {
        // Chunked bodies are refused rather than parsed. Nothing this service accepts is large
        // enough to need streaming, and a second framing path is a second place to get length
        // handling wrong.
        return Err(ApiError::new(
            411,
            "length_required",
            "the request body must be sent with a content-length",
            false,
        ));
    }
    let Some((_, value)) = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
    else {
        return Ok(Vec::new());
    };
    let length: usize = value
        .parse()
        .map_err(|_| ApiError::invalid("the content-length header is not a length"))?;
    if length > MAX_BODY_BYTES {
        return Err(ApiError::new(
            413,
            "body_limit",
            "the request body exceeded the service's declared allowance",
            false,
        ));
    }
    let mut body = vec![0_u8; length];
    reader
        .read_exact(&mut body)
        .map_err(|_| ApiError::invalid("the request body was shorter than its content-length"))?;
    Ok(body)
}

/// Reads one CRLF-terminated line, refusing anything longer than `limit`.
fn read_line(reader: &mut impl BufRead, limit: usize) -> Result<String, ApiError> {
    let mut raw = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        if reader.read(&mut byte).map_err(|_| unreadable())? == 0 {
            return Err(unreadable());
        }
        if byte[0] == b'\n' {
            if raw.last() == Some(&b'\r') {
                raw.pop();
            }
            return String::from_utf8(raw)
                .map_err(|_| ApiError::invalid("the request contained non-UTF-8 header bytes"));
        }
        if raw.len() == limit {
            return Err(ApiError::new(
                431,
                "header_limit",
                "a request line exceeded the service's declared allowance",
                false,
            ));
        }
        raw.push(byte[0]);
    }
}

fn unreadable() -> ApiError {
    ApiError::invalid("the request ended before it was complete")
}
