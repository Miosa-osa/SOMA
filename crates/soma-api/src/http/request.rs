use std::io::BufRead;

use crate::envelope::ApiError;

/// Largest accepted request line, generous for a path of bounded identifiers.
pub const MAX_REQUEST_LINE_BYTES: usize = 8 * 1024;
/// Largest accepted single header line.
pub const MAX_HEADER_LINE_BYTES: usize = 8 * 1024;
/// Largest accepted header count.
pub const MAX_HEADERS: usize = 64;
/// Largest accepted body, sized to admit a maximal direct command plus its JSON framing.
pub const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

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
        let (method, path) = request_line(&line)?;
        let headers = read_headers(reader)?;
        let body = read_body(reader, &headers)?;
        Ok(Self {
            method,
            path,
            headers,
            body,
        })
    }
}

fn request_line(line: &str) -> Result<(Method, String), ApiError> {
    let mut parts = line.split(' ');
    let (Some(method), Some(target), Some(version)) = (parts.next(), parts.next(), parts.next())
    else {
        return Err(ApiError::invalid("the request line is malformed"));
    };
    if parts.next().is_some() || !version.starts_with("HTTP/1.") {
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
