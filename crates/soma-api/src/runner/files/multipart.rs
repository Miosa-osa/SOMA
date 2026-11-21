//! A streaming reader of one `multipart/form-data` body (RFC 7578 over RFC 2046).
//!
//! An upload's file can be larger than anything this process should hold, so the body is read as
//! it arrives: each chunk fed in comes back as the events it completes, and a part's bytes are
//! handed on as soon as they cannot be the start of the next delimiter. What is held at any time
//! is one chunk and less than one delimiter of look-behind, whatever the size of the file.
//!
//! The body is hostile input. A part's header block is bounded, a part must name itself with a
//! `Content-Disposition: form-data` header, and a body that ends before its closing delimiter is
//! refused rather than taken as complete.

/// The longest header block one part may carry.
const MAX_PART_HEADER_BYTES: usize = 16 * 1024;
/// The longest boundary RFC 2046 allows.
const MAX_BOUNDARY_BYTES: usize = 70;

/// What a fed chunk completed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Event {
    /// A part begins, with its form name and the filename it carried, if any.
    Begin {
        name: String,
        filename: Option<String>,
    },
    /// Bytes of the part that began last.
    Data(Vec<u8>),
    /// The part that began last has ended.
    End,
}

/// Why the body is not a multipart body this reader accepts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Malformed;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    /// Before the first delimiter.
    Preamble,
    /// Just after a delimiter: either the closing `--` or the line end before headers.
    Delimited,
    Headers,
    Part,
    /// After the closing delimiter; anything further is epilogue and is ignored.
    Done,
}

/// One body being read.
#[derive(Debug)]
pub(crate) struct Multipart {
    delimiter: Vec<u8>,
    held: Vec<u8>,
    state: State,
}

impl Multipart {
    /// A reader for the boundary `content_type` names, or `None` when it names none.
    pub(crate) fn for_content_type(content_type: &str) -> Option<Self> {
        let (kind, parameters) = content_type.split_once(';')?;
        if !kind.trim().eq_ignore_ascii_case("multipart/form-data") {
            return None;
        }
        let boundary = parameters.split(';').find_map(|parameter| {
            let (name, value) = parameter.split_once('=')?;
            name.trim()
                .eq_ignore_ascii_case("boundary")
                .then(|| value.trim().trim_matches('"').to_owned())
        })?;
        if boundary.is_empty() || boundary.len() > MAX_BOUNDARY_BYTES {
            return None;
        }
        let mut delimiter = b"\r\n--".to_vec();
        delimiter.extend_from_slice(boundary.as_bytes());
        // The first delimiter may open the body with no line end before it, so the reader acts
        // as though one was there.
        Some(Self {
            delimiter,
            held: b"\r\n".to_vec(),
            state: State::Preamble,
        })
    }

    /// Feeds one chunk and appends the events it completed.
    ///
    /// # Errors
    ///
    /// Returns [`Malformed`] for a part without a form name or with an oversized header block, or
    /// for bytes after a delimiter that are neither a line end nor the closing `--`.
    pub(crate) fn feed(&mut self, chunk: &[u8], events: &mut Vec<Event>) -> Result<(), Malformed> {
        if self.state == State::Done {
            return Ok(());
        }
        self.held.extend_from_slice(chunk);
        while self.step(events)? {}
        Ok(())
    }

    /// Whether the body reached its closing delimiter.
    pub(crate) fn finished(&self) -> bool {
        self.state == State::Done
    }

    /// Takes one step; `false` when more input is needed.
    fn step(&mut self, events: &mut Vec<Event>) -> Result<bool, Malformed> {
        match self.state {
            State::Preamble => {
                if let Some(at) = find(&self.held, &self.delimiter) {
                    self.held.drain(..at + self.delimiter.len());
                    self.state = State::Delimited;
                    Ok(true)
                } else {
                    self.keep_tail();
                    Ok(false)
                }
            }
            State::Delimited => self.after_delimiter(),
            State::Headers => {
                let Some(at) = find(&self.held, b"\r\n\r\n") else {
                    return if self.held.len() > MAX_PART_HEADER_BYTES {
                        Err(Malformed)
                    } else {
                        Ok(false)
                    };
                };
                if at > MAX_PART_HEADER_BYTES {
                    return Err(Malformed);
                }
                let block = String::from_utf8(self.held[..at].to_vec()).map_err(|_| Malformed)?;
                self.held.drain(..at + 4);
                let (name, filename) = disposition(&block).ok_or(Malformed)?;
                events.push(Event::Begin { name, filename });
                self.state = State::Part;
                Ok(true)
            }
            State::Part => {
                if let Some(at) = find(&self.held, &self.delimiter) {
                    if at > 0 {
                        events.push(Event::Data(self.held[..at].to_vec()));
                    }
                    events.push(Event::End);
                    self.held.drain(..at + self.delimiter.len());
                    self.state = State::Delimited;
                    Ok(true)
                } else {
                    let safe = self.held.len().saturating_sub(self.delimiter.len() - 1);
                    if safe > 0 {
                        events.push(Event::Data(self.held.drain(..safe).collect()));
                    }
                    Ok(false)
                }
            }
            State::Done => Ok(false),
        }
    }

    /// After a delimiter: optional padding, then `--` to close or a line end to open a part.
    fn after_delimiter(&mut self) -> Result<bool, Malformed> {
        let padding = self
            .held
            .iter()
            .take_while(|byte| **byte == b' ' || **byte == b'\t')
            .count();
        let rest = &self.held[padding..];
        if rest.len() < 2 {
            return Ok(false);
        }
        match &rest[..2] {
            b"--" => {
                self.state = State::Done;
                self.held.clear();
                Ok(false)
            }
            b"\r\n" => {
                self.held.drain(..padding + 2);
                self.state = State::Headers;
                Ok(true)
            }
            _ => Err(Malformed),
        }
    }

    /// Keeps only what could still be the start of a delimiter.
    fn keep_tail(&mut self) {
        let keep = self.delimiter.len() - 1;
        if self.held.len() > keep {
            self.held.drain(..self.held.len() - keep);
        }
    }
}

/// The form name and filename one part's header block gives it.
fn disposition(block: &str) -> Option<(String, Option<String>)> {
    let line = block.split("\r\n").find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("content-disposition")
            .then_some(value)
    })?;
    let mut parameters = split_parameters(line).into_iter();
    if !parameters.next()?.eq_ignore_ascii_case("form-data") {
        return None;
    }
    let (mut name, mut filename) = (None, None);
    for parameter in parameters {
        let Some((key, value)) = parameter.split_once('=') else {
            continue;
        };
        let value = unquote(value.trim());
        match key.trim().to_ascii_lowercase().as_str() {
            "name" => name = Some(value),
            "filename" => filename = Some(value),
            _ => {}
        }
    }
    Some((name?, filename))
}

/// Splits a header value on `;` outside quoted strings.
fn split_parameters(value: &str) -> Vec<String> {
    let mut parameters = vec![String::new()];
    let (mut quoted, mut escaped) = (false, false);
    for character in value.chars() {
        let current = parameters.last_mut().expect("never empty");
        match character {
            _ if escaped => {
                current.push(character);
                escaped = false;
            }
            '\\' if quoted => {
                current.push(character);
                escaped = true;
            }
            '"' => {
                current.push(character);
                quoted = !quoted;
            }
            ';' if !quoted => parameters.push(String::new()),
            _ => current.push(character),
        }
    }
    parameters
        .iter()
        .map(|part| part.trim().to_owned())
        .collect()
}

/// A token as it is, or a quoted string without its quotes and escapes.
fn unquote(value: &str) -> String {
    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return value.to_owned();
    };
    let mut text = String::with_capacity(inner.len());
    let mut escaped = false;
    for character in inner.chars() {
        if escaped || character != '\\' {
            text.push(character);
            escaped = false;
        } else {
            escaped = true;
        }
    }
    text
}

/// The first position of `needle`, found by scanning for its first byte, which in a file body is
/// rare enough that the scan, not the comparison, is the cost.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    let first = *needle.first()?;
    let last_start = haystack.len().checked_sub(needle.len())?;
    let mut from = 0;
    while from <= last_start {
        let at = from
            + haystack[from..=last_start]
                .iter()
                .position(|byte| *byte == first)?;
        if haystack[at..].starts_with(needle) {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

#[cfg(test)]
#[path = "multipart_tests.rs"]
mod tests;
