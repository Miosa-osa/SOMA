//! A `/filesystem/write` body read as it arrives, with its file decoded on the way through.
//!
//! The body is soma-api's own `FilesystemBody`: one JSON object whose `content` is the file as
//! base64. A file larger than the runner should hold cannot be parsed whole, so this reader walks
//! the object byte by byte: every field other than `content` is captured as its raw JSON text
//! (bounded), and `content` is decoded from base64 as it streams past and handed on in pieces.
//! What is held at any time is one chunk of input, three characters of base64, and the small
//! fields; the decoded file goes to the caller.
//!
//! The fields are checked afterwards exactly as the buffered route checks them, by giving the
//! captured fields to `FilesystemBody` with an empty `content`, so a body this reader accepts is
//! one the buffered route would accept too, and a malformed one is refused with the same answer.

use base64::{Engine as _, engine::general_purpose::STANDARD};

mod raw;

/// The most raw JSON text the fields other than `content` may hold together.
const MAX_FIELD_BYTES: usize = 64 * 1024;

/// Why the body is not one this reader accepts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BodyRefused {
    /// Not one JSON object of string keys, or its fields are larger than the bound.
    Malformed,
    /// `content` is not a base64 string.
    Content,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    Start,
    /// Before a key or the closing brace; `first` when no field has been read yet.
    BeforeKey {
        first: bool,
    },
    Key {
        escaped: bool,
    },
    Colon,
    BeforeValue,
    /// Inside the `content` string.
    Content {
        escaped: bool,
    },
    /// Inside another field's value, kept as raw text.
    Raw(Raw),
    AfterValue,
    Done,
}

/// Where a raw value's scan stands: nested brackets, and whether inside a string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Raw {
    depth: u32,
    in_string: bool,
    escaped: bool,
}

/// One body being read.
#[derive(Debug)]
pub(crate) struct WriteBody {
    state: State,
    key: Vec<u8>,
    value: Vec<u8>,
    /// Every field other than `content`, as `"key":value` JSON text joined by commas.
    fields: Vec<u8>,
    /// The `path` field's text, once it has been read.
    path: Option<String>,
    /// Whether `content` has begun, and whether it has ended.
    content: (bool, bool),
    /// Base64 characters not yet decoded, fewer than four.
    pending: Vec<u8>,
    /// Whether padding has been seen, after which only the closing quote may follow.
    padded: bool,
}

impl WriteBody {
    pub(crate) const fn new() -> Self {
        Self {
            state: State::Start,
            key: Vec::new(),
            value: Vec::new(),
            fields: Vec::new(),
            path: None,
            content: (false, false),
            pending: Vec::new(),
            padded: false,
        }
    }

    /// The `path` field, when it has been read in full.
    pub(crate) fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// Whether `content` has begun.
    pub(crate) const fn content_started(&self) -> bool {
        self.content.0
    }

    /// Feeds one chunk, appending the file bytes it completed to `decoded`.
    ///
    /// # Errors
    ///
    /// Returns why the body cannot be read.
    pub(crate) fn feed(&mut self, chunk: &[u8], decoded: &mut Vec<u8>) -> Result<(), BodyRefused> {
        let mut run_start = None;
        for (index, &byte) in chunk.iter().enumerate() {
            // Plain base64 characters are the bulk of the body and are taken as a run.
            if matches!(self.state, State::Content { escaped: false }) && is_base64(byte) {
                run_start.get_or_insert(index);
                continue;
            }
            if let Some(start) = run_start.take() {
                self.base64(&chunk[start..index], decoded)?;
            }
            self.step(byte, decoded)?;
        }
        if let Some(start) = run_start {
            self.base64(&chunk[start..], decoded)?;
        }
        Ok(())
    }

    /// Ends the body: the other fields as one JSON object with an empty `content`, ready for
    /// `FilesystemBody`, or `None` when the body named no `content` at all.
    ///
    /// # Errors
    ///
    /// Returns [`BodyRefused::Malformed`] for a body that did not end where a JSON object ends.
    pub(crate) fn finish(self) -> Result<Option<Vec<u8>>, BodyRefused> {
        if self.state != State::Done {
            return Err(BodyRefused::Malformed);
        }
        if !self.content.1 {
            return Ok(None);
        }
        let mut object = b"{\"content\":\"\"".to_vec();
        if !self.fields.is_empty() {
            object.push(b',');
            object.extend_from_slice(&self.fields);
        }
        object.push(b'}');
        Ok(Some(object))
    }

    fn step(&mut self, byte: u8, decoded: &mut Vec<u8>) -> Result<(), BodyRefused> {
        let space = byte.is_ascii_whitespace();
        self.state = match self.state {
            State::Start if space => State::Start,
            State::Start if byte == b'{' => State::BeforeKey { first: true },
            State::BeforeKey { .. } if space => self.state,
            State::BeforeKey { first: true } if byte == b'}' => State::Done,
            State::BeforeKey { .. } if byte == b'"' => {
                self.key.clear();
                State::Key { escaped: false }
            }
            State::Key { escaped } => {
                if !escaped && byte == b'"' {
                    State::Colon
                } else {
                    self.key.push(byte);
                    if self.key.len() > 64 {
                        return Err(BodyRefused::Malformed);
                    }
                    State::Key {
                        escaped: !escaped && byte == b'\\',
                    }
                }
            }
            State::Colon if space => State::Colon,
            State::Colon if byte == b':' => State::BeforeValue,
            State::BeforeValue if space => State::BeforeValue,
            State::BeforeValue => self.begin_value(byte)?,
            State::Content { escaped: true } => {
                // `\/` is the one escape a base64 string may carry; it is a `/`.
                if byte != b'/' {
                    return Err(BodyRefused::Content);
                }
                self.base64(b"/", decoded)?;
                State::Content { escaped: false }
            }
            State::Content { escaped: false } => match byte {
                b'\\' => State::Content { escaped: true },
                b'"' => {
                    self.end_content()?;
                    State::AfterValue
                }
                _ => return Err(BodyRefused::Content),
            },
            State::Raw(raw) => self.raw(raw, byte)?,
            State::AfterValue if space => State::AfterValue,
            State::AfterValue if byte == b',' => State::BeforeKey { first: false },
            State::AfterValue if byte == b'}' => State::Done,
            State::Done if space => State::Done,
            _ => return Err(BodyRefused::Malformed),
        };
        Ok(())
    }

    fn begin_value(&mut self, byte: u8) -> Result<State, BodyRefused> {
        if self.key == b"content" {
            if self.content.0 || byte != b'"' {
                return Err(if self.content.0 {
                    BodyRefused::Malformed
                } else {
                    BodyRefused::Content
                });
            }
            self.content.0 = true;
            return Ok(State::Content { escaped: false });
        }
        self.value.clear();
        let raw = Raw {
            depth: 0,
            in_string: false,
            escaped: false,
        };
        self.raw(raw, byte)
    }

    /// Decodes a run of base64 characters, holding back what does not yet make a quartet.
    fn base64(&mut self, run: &[u8], decoded: &mut Vec<u8>) -> Result<(), BodyRefused> {
        if self.padded {
            return Err(BodyRefused::Content);
        }
        let held = self.pending.len();
        let whole = (held + run.len()) / 4 * 4;
        if whole == 0 {
            self.pending.extend_from_slice(run);
            return Ok(());
        }
        let mut quartets = std::mem::take(&mut self.pending);
        quartets.extend_from_slice(&run[..whole - held]);
        self.pending.extend_from_slice(&run[whole - held..]);
        self.padded = quartets.last() == Some(&b'=');
        STANDARD
            .decode_vec(&quartets, decoded)
            .map_err(|_| BodyRefused::Content)
    }

    fn end_content(&mut self) -> Result<(), BodyRefused> {
        if !self.pending.is_empty() {
            return Err(BodyRefused::Content);
        }
        self.content.1 = true;
        Ok(())
    }
}

const fn is_base64(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=')
}

#[cfg(test)]
#[path = "write_body_tests.rs"]
mod tests;
