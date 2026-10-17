//! Turning a chunk of guest bytes into text without cutting a character in half.
//!
//! Guest output arrives in chunks that are not aligned to character boundaries, so decoding each
//! one on its own would turn every split character into two replacement characters.

/// Turns a byte stream into text without cutting a character in half.
///
/// Guest output is bytes. A chunk boundary can fall inside a multi-byte character, and decoding
/// each chunk on its own would turn both halves into replacement characters. This holds an
/// incomplete trailing sequence back until the next chunk, and replaces only bytes that can
/// never become valid. [`Self::finish`] releases whatever is left when the command ends.
#[derive(Default)]
pub(super) struct Utf8Splitter {
    held: Vec<u8>,
}

impl Utf8Splitter {
    pub(super) fn push(&mut self, bytes: &[u8]) -> String {
        self.held.extend_from_slice(bytes);
        let mut text = String::new();
        let mut rest = self.held.as_slice();
        loop {
            match std::str::from_utf8(rest) {
                Ok(valid) => {
                    text.push_str(valid);
                    rest = &[];
                    break;
                }
                Err(error) => {
                    let (valid, after) = rest.split_at(error.valid_up_to());
                    text.push_str(&String::from_utf8_lossy(valid));
                    if let Some(length) = error.error_len() {
                        // Bytes that can never be valid: replace them and carry on.
                        text.push(char::REPLACEMENT_CHARACTER);
                        rest = &after[length..];
                    } else {
                        // The chunk ended inside a character: wait for the rest of it.
                        rest = after;
                        break;
                    }
                }
            }
        }
        self.held = rest.to_vec();
        text
    }

    pub(super) fn finish(&mut self) -> String {
        let text = String::from_utf8_lossy(&self.held).into_owned();
        self.held.clear();
        text
    }
}

#[cfg(test)]
mod tests {
    use super::Utf8Splitter;

    #[test]
    fn a_character_split_across_chunks_is_decoded_once_whole() {
        let mut splitter = Utf8Splitter::default();
        let euro = "\u{20ac}".as_bytes();
        assert_eq!(splitter.push(&euro[..1]), "");
        assert_eq!(splitter.push(&euro[1..2]), "");
        assert_eq!(splitter.push(&euro[2..]), "\u{20ac}");
        assert_eq!(splitter.finish(), "");
    }

    #[test]
    fn plain_text_passes_straight_through() {
        let mut splitter = Utf8Splitter::default();
        assert_eq!(splitter.push(b"hello "), "hello ");
        assert_eq!(splitter.push(b"world\n"), "world\n");
    }

    #[test]
    fn bytes_that_can_never_be_valid_are_replaced_without_waiting() {
        let mut splitter = Utf8Splitter::default();
        assert_eq!(splitter.push(&[b'a', 0xff, b'b']), "a\u{fffd}b");
        // A lone continuation byte is invalid on its own.
        assert_eq!(splitter.push(&[0x80]), "\u{fffd}");
    }

    #[test]
    fn an_incomplete_character_at_the_end_is_released_replaced() {
        let mut splitter = Utf8Splitter::default();
        assert_eq!(splitter.push(&"\u{20ac}".as_bytes()[..2]), "");
        assert_eq!(splitter.finish(), "\u{fffd}");
        assert_eq!(
            splitter.finish(),
            "",
            "finishing twice releases nothing more"
        );
    }

    #[test]
    fn a_character_cut_at_every_possible_boundary_survives() {
        let text = "a\u{e9}\u{20ac}\u{1f600}z";
        let bytes = text.as_bytes();
        for cut in 0..=bytes.len() {
            let mut splitter = Utf8Splitter::default();
            let mut out = splitter.push(&bytes[..cut]);
            out.push_str(&splitter.push(&bytes[cut..]));
            out.push_str(&splitter.finish());
            assert_eq!(out, text, "cut at {cut}");
        }
    }
}
