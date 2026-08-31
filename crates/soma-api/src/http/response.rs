use std::io::{self, Write};

/// One HTTP response: a status and a JSON body.
///
/// The service has exactly one media type, so there is no content negotiation and no header map.
/// Anything a caller could vary here would be a promise the service does not keep.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Response {
    #[must_use]
    pub const fn new(status: u16, body: Vec<u8>) -> Self {
        Self { status, body }
    }

    /// Writes the response as HTTP/1.1 and closes the exchange.
    ///
    /// The connection is always closed after one response. Keep-alive would require tracking
    /// per-connection framing state for no benefit at this size, and a client that pipelines
    /// against a server that mishandles it fails in ways that are hard to see.
    ///
    /// # Errors
    ///
    /// Returns the underlying write failure.
    pub fn write_to(&self, writer: &mut impl Write) -> io::Result<()> {
        write!(
            writer,
            "HTTP/1.1 {} {}\r\n\
             content-type: application/json\r\n\
             content-length: {}\r\n\
             cache-control: no-store\r\n\
             connection: close\r\n\r\n",
            self.status,
            reason(self.status),
            self.body.len(),
        )?;
        writer.write_all(&self.body)?;
        writer.flush()
    }
}

/// The reason phrase for each status this service emits.
///
/// Only the statuses the service actually produces are named. An unknown status falling back to
/// a generic phrase is preferable to a table that pretends to cover codes no route returns.
const fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Error",
    }
}
