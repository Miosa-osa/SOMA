//! The fields other than `content`, kept as their raw JSON text.
//!
//! A raw value ends at the first byte that cannot continue it: the closing quote of a string,
//! the bracket that closes the outermost object or array, or the comma, brace, or space after a
//! number or literal. Its text is checked later, by `FilesystemBody`, so this only has to find
//! where it ends.

use super::{BodyRefused, MAX_FIELD_BYTES, Raw, State, WriteBody};

impl WriteBody {
    /// Scans one byte of a raw value; the value ends at the first byte that cannot continue it.
    pub(super) fn raw(&mut self, mut raw: Raw, byte: u8) -> Result<State, BodyRefused> {
        let scalar_end = raw.depth == 0
            && !raw.in_string
            && !self.value.is_empty()
            && self.value[0] != b'"'
            && matches!(byte, b',' | b'}' | b' ' | b'\t' | b'\r' | b'\n');
        if scalar_end {
            self.end_raw()?;
            return Ok(after_scalar(byte));
        }
        self.value.push(byte);
        if self.value.len() + self.fields.len() > MAX_FIELD_BYTES {
            return Err(BodyRefused::Malformed);
        }
        if raw.in_string {
            if raw.escaped {
                raw.escaped = false;
            } else if byte == b'\\' {
                raw.escaped = true;
            } else if byte == b'"' {
                raw.in_string = false;
            }
        } else {
            match byte {
                b'"' => raw.in_string = true,
                b'{' | b'[' => raw.depth += 1,
                b'}' | b']' => {
                    raw.depth = raw.depth.checked_sub(1).ok_or(BodyRefused::Malformed)?
                }
                _ => {}
            }
        }
        let closed = raw.depth == 0 && !raw.in_string && matches!(byte, b'"' | b'}' | b']');
        if closed && self.value.len() > 1 {
            self.end_raw()?;
            return Ok(State::AfterValue);
        }
        Ok(State::Raw(raw))
    }

    fn end_raw(&mut self) -> Result<(), BodyRefused> {
        let key = std::str::from_utf8(&self.key).map_err(|_| BodyRefused::Malformed)?;
        if key == "path" {
            let path: String =
                serde_json::from_slice(&self.value).map_err(|_| BodyRefused::Malformed)?;
            self.path = Some(path);
        }
        if !self.fields.is_empty() {
            self.fields.push(b',');
        }
        self.fields.push(b'"');
        self.fields.extend_from_slice(&self.key);
        self.fields.extend_from_slice(b"\":");
        self.fields.extend_from_slice(&self.value);
        Ok(())
    }
}

/// Where the object stands after the byte that ended a number or a literal.
const fn after_scalar(byte: u8) -> State {
    match byte {
        b',' => State::BeforeKey { first: false },
        b'}' => State::Done,
        _ => State::AfterValue,
    }
}
